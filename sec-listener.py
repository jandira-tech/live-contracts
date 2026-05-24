#!/usr/bin/env python3
"""
Continuous RSS Listener for SEC Filings with EX-10 Exhibit Extraction

This script:
1. Listens to SEC RSS feed for new filings
2. Uses EFTS to fetch full submission details
3. Extracts and saves EX-10 exhibits as they're published
4. Persists seen accession numbers to avoid duplicates
"""

import asyncio
import aiohttp
import sqlite3
import json
import time
import xml.etree.ElementTree as ET
import re
import os
import platform
from datetime import datetime, timedelta
from collections import Counter
from datamule import Submission, format_accession

# Configuration
RSS_FEED_URL = 'https://www.sec.gov/cgi-bin/browse-edgar?action=getcurrent&output=rss'
EFTS_BASE_URL = 'https://efts.sec.gov/LATEST/search-index'
HEADERS = {'User-Agent': 'SEC Monitor Bot sec-monitor@example.com'}

# Rate limiting
REQUESTS_PER_SECOND = 5  # Below SEC's 10 req/sec limit
MIN_REQUEST_INTERVAL = 1.0 / REQUESTS_PER_SECOND

# Alert configuration
ALERT_SOUND_ENABLED = False
ALERT_FILE_ENABLED = True
ALERT_FILE_PATH = 'ex10_alerts.log'

# Runtime configuration
RUN_DURATION_HOURS = 24  # Run for 24 hours then exit

class EX10RSSListener:
    def __init__(self, db_path='ex10_listener.db'):
        self.db_path = db_path
        self.last_request_time = 0
        self.session = None
        self.running = False
        self.start_time = None
        self.last_alert_time = 0
        self.alert_cooldown = 300  # 5 minutes between alerts for same filing

        # Initialize database
        self._init_db()

        # Create alerts directory if it doesn't exist
        if ALERT_FILE_ENABLED:
            os.makedirs(os.path.dirname(ALERT_FILE_PATH) if os.path.dirname(ALERT_FILE_PATH) else '.', exist_ok=True)

    def _init_db(self):
        """Initialize SQLite database for persistence"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()

            # Table for seen accession numbers
            cursor.execute('''
                CREATE TABLE IF NOT EXISTS seen_accessions (
                    accession TEXT PRIMARY KEY,
                    first_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    form_type TEXT,
                    cik TEXT
                )
            ''')

            # Table for traditional EX-10 exhibits found (EX-10, EX-10.1, EX-10.2, etc.)
            # EXCLUDES XBRL exhibits like EX-101, EX-100, etc.
            cursor.execute('''
                CREATE TABLE IF NOT EXISTS ex10_exhibits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    accession TEXT,
                    cik TEXT,
                    form_type TEXT,
                    doc_type TEXT,
                    filename TEXT,
                    description TEXT,
                    sequence TEXT,
                    filing_url TEXT,
                    found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    UNIQUE(accession, doc_type, filename)
                )
            ''')

            # Table for all exhibits (not just EX-10)
            cursor.execute('''
                CREATE TABLE IF NOT EXISTS all_exhibits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    accession TEXT,
                    cik TEXT,
                    form_type TEXT,
                    doc_type TEXT,
                    filename TEXT,
                    description TEXT,
                    sequence TEXT,
                    filing_url TEXT,
                    found_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
            ''')

            # Table for RSS feed entries
            cursor.execute('''
                CREATE TABLE IF NOT EXISTS rss_entries (
                    accession TEXT PRIMARY KEY,
                    cik TEXT,
                    form_type TEXT,
                    filing_date TEXT,
                    rss_summary TEXT,
                    processed BOOLEAN DEFAULT FALSE,
                    first_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
            ''')

            conn.commit()

    def _rate_limit(self):
        """Enforce rate limiting"""
        now = time.time()
        elapsed = now - self.last_request_time
        if elapsed < MIN_REQUEST_INTERVAL:
            time.sleep(MIN_REQUEST_INTERVAL - elapsed)
        self.last_request_time = time.time()

    def _play_alert_sound(self):
        """Play an alert sound to wake you up"""
        if not ALERT_SOUND_ENABLED:
            return

        try:
            if platform.system() == 'Windows':
                import winsound
                winsound.Beep(1000, 1000)
                winsound.Beep(1000, 1000)
            elif platform.system() == 'Darwin':
                os.system('afplay /System/Library/Sounds/Ping.aiff')
                time.sleep(0.5)
                os.system('afplay /System/Library/Sounds/Ping.aiff')
            else:
                try:
                    os.system('paplay /usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga')
                    time.sleep(0.5)
                    os.system('paplay /usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga')
                except:
                    print('\a\a\a')
                    time.sleep(0.5)
                    print('\a\a\a')
        except Exception as e:
            print(f"Could not play alert sound: {e}")

    def _log_alert(self, message):
        """Log alert to file"""
        if not ALERT_FILE_ENABLED:
            return

        try:
            timestamp = datetime.now().strftime('%Y-%m-%d %H:%M:%S')
            alert_message = f"[{timestamp}] ALERT: {message}\n"

            with open(ALERT_FILE_PATH, 'a') as f:
                f.write(alert_message)

            print(f"🚨 {alert_message.strip()}")
        except Exception as e:
            print(f"Could not log alert: {e}")

    def _trigger_alert(self, accession, cik, form_type, doc_type, filename):
        """Trigger alert for new EX-10 exhibit"""
        now = time.time()

        if now - self.last_alert_time < self.alert_cooldown:
            return

        self.last_alert_time = now

        alert_message = f"NEW TRADITIONAL EX-10 EXHIBIT! Accession: {accession}, CIK: {cik}, Form: {form_type}, Type: {doc_type}, File: {filename}"

        self._log_alert(alert_message)
        self._play_alert_sound()

        print("\n" + "=" * 80)
        print("🚨 WAKE UP! NEW TRADITIONAL EX-10 EXHIBIT FOUND! 🚨")
        print("=" * 80)
        print(f"📋 Accession: {accession}")
        print(f"🏢 CIK: {cik}")
        print(f"📄 Form Type: {form_type}")
        print(f"🏷️  Exhibit Type: {doc_type}")
        print(f"📁 Filename: {filename}")
        print(f"⏰ Time: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
        print("=" * 80 + "\n")

    async def _ensure_session(self):
        """Ensure we have an active aiohttp session"""
        if self.session is None or self.session.closed:
            self.session = aiohttp.ClientSession(headers=HEADERS)
        return self.session

    def _is_accession_seen(self, accession):
        """Check if accession has already been processed"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute('SELECT 1 FROM seen_accessions WHERE accession = ?', (accession,))
            return cursor.fetchone() is not None

    def _mark_accession_seen(self, accession, form_type, cik):
        """Mark accession as seen"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute('''
                INSERT OR IGNORE INTO seen_accessions (accession, form_type, cik)
                VALUES (?, ?, ?)
            ''', (accession, form_type, cik))
            conn.commit()

    def _save_ex10_exhibit(self, exhibit_data):
        """Save EX-10 exhibit to database"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute('''
                INSERT OR IGNORE INTO ex10_exhibits
                (accession, cik, form_type, doc_type, filename, description, sequence, filing_url)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ''', (
                exhibit_data['accession'],
                exhibit_data['cik'],
                exhibit_data['form_type'],
                exhibit_data['doc_type'],
                exhibit_data['filename'],
                exhibit_data['description'],
                exhibit_data['sequence'],
                exhibit_data['url']
            ))
            conn.commit()

    def _save_all_exhibit(self, exhibit_data):
        """Save any exhibit to database"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute('''
                INSERT INTO all_exhibits
                (accession, cik, form_type, doc_type, filename, description, sequence, filing_url)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ''', (
                exhibit_data['accession'],
                exhibit_data['cik'],
                exhibit_data['form_type'],
                exhibit_data['doc_type'],
                exhibit_data['filename'],
                exhibit_data['description'],
                exhibit_data['sequence'],
                exhibit_data['url']
            ))
            conn.commit()

    def _save_rss_entry(self, accession, cik, form_type, filing_date, summary):
        """Save RSS entry to database"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute('''
                INSERT OR IGNORE INTO rss_entries
                (accession, cik, form_type, filing_date, rss_summary)
                VALUES (?, ?, ?, ?, ?)
            ''', (accession, cik, form_type, filing_date, summary))
            conn.commit()

    def _parse_rss_feed(self, rss_content):
        """Parse SEC RSS feed and extract filing information"""
        root = ET.fromstring(rss_content)
        namespace = {'atom': 'http://www.w3.org/2005/Atom'}
        entries = root.findall('atom:entry', namespace)

        filings = []
        for entry in entries:
            try:
                url = entry.find('atom:link', namespace).get('href')

                # Extract accession number
                accession_match = re.search(r'/(\d{10})-(\d{2})-(\d{6})', url)
                if accession_match:
                    accession = f"{accession_match.group(1)}-{accession_match.group(2)}-{accession_match.group(3)}"
                else:
                    continue

                # Extract CIK
                cik_match = re.search(r'/data/(\d+)/', url)
                cik = cik_match.group(1) if cik_match else ''

                # Extract form type
                form_type = entry.find('atom:category', namespace).get('term', '')

                # Extract filing date from summary
                summary_text = entry.find('atom:summary', namespace).text or ''
                filing_date_match = re.search(r'Filed:</b>\s*(\d{4}-\d{2}-\d{2})', summary_text)
                filing_date = filing_date_match.group(1) if filing_date_match else ''

                filings.append({
                    'accession': accession,
                    'cik': cik,
                    'form_type': form_type,
                    'filing_date': filing_date,
                    'url': url,
                    'summary': summary_text
                })

            except Exception as e:
                print(f"Error parsing RSS entry: {e}")
                continue

        return filings

    async def _fetch_submission_via_efts(self, accession, cik, form_type):
        """Fetch submission details using EFTS API"""
        self._rate_limit()

        try:
            clean_accession = accession.replace('-', '')
            params = {
                'q': f'adsh:"{clean_accession}"',
                'from': '0',
                'size': '1'
            }

            async with aiohttp.ClientSession(headers=HEADERS) as session:
                async with session.get(EFTS_BASE_URL, params=params) as response:
                    if response.status == 200:
                        data = await response.json()
                        hits = data.get('hits', {}).get('hits', [])

                        if hits:
                            source = hits[0]['_source']
                            return {
                                'accession': accession,
                                'cik': cik,
                                'form_type': form_type,
                                'filing_date': source.get('file_date', ''),
                                'primary_document_url': source.get('primary_document', '')
                            }
        except Exception as e:
            print(f"Error fetching EFTS data for {accession}: {e}")

        return None

    async def _load_submission_and_extract_exhibits(self, accession, cik, form_type):
        """Load submission and extract EX-10 exhibits"""
        try:
            formatted_accession = format_accession(accession.replace('-', ''), 'dash')
            url = f"https://www.sec.gov/Archives/edgar/data/{cik}/{formatted_accession}.txt"

            sub = Submission(url=url)
            documents = sub.metadata.content.get('documents', [])

            ex10_docs = []
            other_ex_docs = []

            for doc in documents:
                doc_type = doc.get('type', '')

                if doc_type.startswith('EX-10'):
                    suffix = doc_type[5:]
                    if suffix == '' or suffix.startswith('.'):
                        ex10_docs.append(doc)
                    else:
                        other_ex_docs.append(doc)
                elif doc_type.startswith('EX-'):
                    other_ex_docs.append(doc)

            return ex10_docs, other_ex_docs, url

        except Exception as e:
            print(f"Error loading submission {accession}: {e}")
            return [], [], None

    async def _process_filing(self, filing):
        """Process a single filing from RSS feed"""
        accession = filing['accession']
        cik = filing['cik']
        form_type = filing['form_type']

        if self._is_accession_seen(accession):
            return 0, 0

        self._mark_accession_seen(accession, form_type, cik)
        self._save_rss_entry(accession, cik, form_type, filing.get('filing_date', ''), filing.get('summary', ''))

        print(f"🔍 Processing new filing: {accession} ({form_type}) from CIK {cik}")

        ex10_docs, other_ex_docs, filing_url = await self._load_submission_and_extract_exhibits(
            accession, cik, form_type
        )

        ex10_count = 0
        other_count = 0

        for doc in ex10_docs:
            exhibit_data = {
                'accession': accession,
                'cik': cik,
                'form_type': form_type,
                'doc_type': doc.get('type', ''),
                'filename': doc.get('filename', ''),
                'description': doc.get('description', ''),
                'sequence': doc.get('sequence', ''),
                'url': filing_url
            }
            self._save_ex10_exhibit(exhibit_data)
            ex10_count += 1
            print(f"  ✅ Found TRADITIONAL EX-10: {doc.get('type', '')} - {doc.get('filename', '')}")

            self._trigger_alert(
                accession=accession,
                cik=cik,
                form_type=form_type,
                doc_type=doc.get('type', ''),
                filename=doc.get('filename', '')
            )

        for doc in other_ex_docs:
            exhibit_data = {
                'accession': accession,
                'cik': cik,
                'form_type': form_type,
                'doc_type': doc.get('type', ''),
                'filename': doc.get('filename', ''),
                'description': doc.get('description', ''),
                'sequence': doc.get('sequence', ''),
                'url': filing_url
            }
            self._save_all_exhibit(exhibit_data)
            other_count += 1

        return ex10_count, other_count

    async def _poll_rss_feed(self):
        """Poll the SEC RSS feed with full pagination to capture all filings."""
        all_filings = []
        seen_accessions = set()
        start = 0
        page_count = 0

        self._rate_limit()

        try:
            async with aiohttp.ClientSession(headers=HEADERS) as session:
                while True:
                    url = f"{RSS_FEED_URL}&count=100&start={start}"
                    page_count += 1

                    async with session.get(url) as response:
                        if response.status != 200:
                            print(f"⚠️ RSS feed returned status {response.status} at start={start}")
                            break

                        content = await response.text()
                        filings = self._parse_rss_feed(content)

                        if not filings:
                            break

                        # Deduplicate within this poll cycle
                        new_count = 0
                        for f in filings:
                            acc = f.get('accession', '')
                            if acc and acc not in seen_accessions:
                                seen_accessions.add(acc)
                                all_filings.append(f)
                                new_count += 1

                        print(f"📄 RSS page {page_count}: {len(filings)} entries ({new_count} new), start={start}")

                        # If we got fewer than 100, we've reached the last page
                        if len(filings) < 100:
                            break

                        start += 100

                        # Rate limit between pages
                        await asyncio.sleep(0.25)
        except Exception as e:
            print(f"Error polling RSS feed: {e}")

        print(f"📡 RSS Feed: {page_count} pages, {len(all_filings)} total filings")
        return all_filings

    async def _run_monitoring_loop(self, poll_interval=60):
        """Main monitoring loop"""
        self.running = True
        self.start_time = time.time()
        total_ex10 = 0
        total_other = 0

        print(f"🚀 Starting Traditional EX-10 RSS Listener (polling every {poll_interval}s)")
        print(f"📁 Database: {self.db_path}")
        print(f"🎯 Target: EX-10, EX-10.1, EX-10.2, etc. (traditional exhibits only)")
        print(f"❌ Excluded: EX-101, EX-100, EX-102, etc. (XBRL and other series)")
        print(f"⏰ Runtime: {RUN_DURATION_HOURS} hours")
        print(f"🚨 Alerts: Sound={ALERT_SOUND_ENABLED}, File={ALERT_FILE_ENABLED}")
        print("=" * 60)

        while self.running:
            try:
                elapsed_hours = (time.time() - self.start_time) / 3600
                if elapsed_hours >= RUN_DURATION_HOURS:
                    print(f"\n⏰ Runtime limit reached ({RUN_DURATION_HOURS} hours). Stopping...")
                    self.running = False
                    break

                remaining_hours = RUN_DURATION_HOURS - elapsed_hours
                print(f"⏳ Running for {elapsed_hours:.1f}h, {remaining_hours:.1f}h remaining")

                filings = await self._poll_rss_feed()

                if filings:
                    print(f"📡 RSS Feed: Found {len(filings)} new filings")

                    for filing in filings:
                        ex10_count, other_count = await self._process_filing(filing)
                        total_ex10 += ex10_count
                        total_other += other_count

                    if ex10_count > 0 or other_count > 0:
                        print(f"📊 Session totals: {total_ex10} EX-10 exhibits, {total_other} other exhibits")
                        print("-" * 40)
                else:
                    print(f"📡 RSS Feed: No new filings found")

                await asyncio.sleep(poll_interval)

            except KeyboardInterrupt:
                self.running = False
                break
            except Exception as e:
                print(f"❌ Error in monitoring loop: {e}")
                await asyncio.sleep(10)

        if self.session:
            await self.session.close()

        print(f"🎉 Listener stopped. Total: {total_ex10} EX-10 exhibits, {total_other} other exhibits")

    def start(self, poll_interval=60):
        """Start the RSS listener"""
        asyncio.run(self._run_monitoring_loop(poll_interval))

    def get_statistics(self):
        """Get statistics from the database"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()

            cursor.execute('SELECT COUNT(*) FROM ex10_exhibits')
            total_ex10 = cursor.fetchone()[0]

            cursor.execute('SELECT COUNT(*) FROM all_exhibits')
            total_other = cursor.fetchone()[0]

            cursor.execute('SELECT COUNT(*) FROM seen_accessions')
            total_accessions = cursor.fetchone()[0]

            cursor.execute('''
                SELECT doc_type, COUNT(*) as count
                FROM ex10_exhibits
                GROUP BY doc_type
                ORDER BY count DESC
            ''')
            ex10_types = cursor.fetchall()

            cursor.execute('''
                SELECT COUNT(*) FROM ex10_exhibits
                WHERE found_at >= datetime('now', '-1 hour')
            ''')
            recent_ex10 = cursor.fetchone()[0]

            return {
                'total_ex10_exhibits': total_ex10,
                'total_other_exhibits': total_other,
                'total_accessions_processed': total_accessions,
                'ex10_types': dict(ex10_types),
                'recent_ex10': recent_ex10
            }

    def export_ex10_exhibits(self, output_file='ex10_exhibits_export.json'):
        """Export all EX-10 exhibits to JSON file"""
        with sqlite3.connect(self.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute('''
                SELECT accession, cik, form_type, doc_type, filename,
                       description, sequence, filing_url, found_at
                FROM ex10_exhibits
                ORDER BY found_at DESC
            ''')

            exhibits = []
            for row in cursor.fetchall():
                exhibits.append({
                    'accession': row[0],
                    'cik': row[1],
                    'form_type': row[2],
                    'doc_type': row[3],
                    'filename': row[4],
                    'description': row[5],
                    'sequence': row[6],
                    'filing_url': row[7],
                    'found_at': row[8]
                })

            with open(output_file, 'w') as f:
                json.dump(exhibits, f, indent=2)

            print(f"💾 Exported {len(exhibits)} EX-10 exhibits to {output_file}")
            return exhibits

if __name__ == "__main__":
    listener = EX10RSSListener()

    try:
        listener.start(poll_interval=60)
    except KeyboardInterrupt:
        pass
    except Exception as e:
        print(f"\n❌ Listener stopped due to error: {e}")
    finally:
        stats = listener.get_statistics()
        print("\n" + "=" * 60)
        print("📊 FINAL STATISTICS")
        print("=" * 60)
        print(f"Total traditional EX-10 exhibits found: {stats['total_ex10_exhibits']}")
        print(f"Total other exhibits found: {stats['total_other_exhibits']}")
        print(f"Total accessions processed: {stats['total_accessions_processed']}")

        if stats['ex10_types']:
            print(f"\nTraditional EX-10 exhibit types:")
            for doc_type, count in stats['ex10_types'].items():
                print(f"  {doc_type}: {count}")

        listener.export_ex10_exhibits()

        if ALERT_FILE_ENABLED and os.path.exists(ALERT_FILE_PATH):
            print(f"\n📋 Alert log saved to: {ALERT_FILE_PATH}")
