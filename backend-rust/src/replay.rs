//! `sec-ex10-rust replay --from YYYY-MM-DD --to YYYY-MM-DD`: backfill a date range.
//!
//! The live monitor only sees recent filings, so a gap (2026-08-07 onward, while
//! ingest was rejected) has to be replayed from SEC's daily form indexes:
//!
//! 1. `daily-index/YYYY/QTRn/form.YYYYMMDD.idx` lists every filing of the day;
//!    keep the form types that carry material contracts.
//! 2. For each, fetch the small `-index.htm` page and keep only filings that list
//!    an EX-10 document (not EX-101 XBRL), so the multi-megabyte SGML is fetched
//!    only when there is something to extract.
//! 3. Run the same pipeline and delivery as the live producer (outbox included),
//!    with `detected_at` set to SEC's acceptance time so replayed rows do not
//!    pose as today's filings.
//!
//! Progress is kept in a SQLite store (`--state`), so a stopped replay resumes.
//! Requests are paced at `--rps` (default 2.5) to leave room under SEC's 10/s for
//! the live producer running at the same time.
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use once_cell::sync::Lazy;
use regex::Regex;

static INDEX_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?P<form>\S.*?)\s{2,}(?P<company>\S.*?)\s{2,}(?P<cik>\d+)\s+(?P<date>\d{8})\s+edgar/data/\d+/(?P<acc>\d{10}-\d{2}-\d{6})\.txt\s*$").unwrap()
});
// A document type cell: EX-10, EX-10.1, EX-10.01, EX-10(a)… but not EX-101.SCH.
static EX10_CELL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"<td scope="row">\s*EX-10(?:[^0-9<][^<]*)?\s*</td>"#).unwrap());
static ACCEPTED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)Accepted</div>\s*<div class="info">(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})</div>"#).unwrap()
});

/// Base form types (amendments `/A` included) that file material contracts as EX-10.
const WANTED: &[&str] = &[
    "8-K", "10-Q", "10-K", "10-KT", "10-QT", "S-1", "S-4", "S-11", "F-1", "F-4", "20-F",
    "10-12B", "10-12G",
];

/// One line of a daily form index.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexEntry {
    pub form: String,
    pub company: String,
    pub cik: u64,
    /// YYYYMMDD
    pub date_filed: String,
    pub accession: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayArgs {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub rps: f64,
    pub dry_run: bool,
    pub state: String,
}

/// A filing worth processing: listed in the day's index, of a wanted form, not
/// yet done in an earlier run, and with an EX-10 in its filing index.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub entry: IndexEntry,
    pub accepted: DateTime<Utc>,
}

#[derive(Debug, Default, PartialEq)]
pub struct DayReport {
    pub listed: usize,
    pub wanted: usize,
    pub already_done: usize,
    pub without_ex10: usize,
    pub index_errors: usize,
    pub candidates: usize,
}

/// Paces requests to SEC: at most one start per `interval`, shared by every caller.
pub struct Pacer {
    interval: std::time::Duration,
    last: tokio::sync::Mutex<Option<std::time::Instant>>,
}

impl Pacer {
    pub fn new(rps: f64) -> Self {
        Self {
            interval: std::time::Duration::from_secs_f64(1.0 / rps.max(0.01)),
            last: tokio::sync::Mutex::new(None),
        }
    }

    pub async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.interval {
                tokio::time::sleep(self.interval - elapsed).await;
            }
        }
        *last = Some(std::time::Instant::now());
    }
}

pub struct Lister<'a> {
    pub client: &'a reqwest::Client,
    /// "https://www.sec.gov"; a local server in tests.
    pub sec_base: String,
    pub pacer: &'a Pacer,
    pub store: &'a crate::store::Store,
}

impl Lister<'_> {
    /// List one day's candidates. Filings without an EX-10 are recorded as done
    /// here; candidates are recorded by the caller once their rows are delivered,
    /// and a filing whose index could not be fetched is left for the next run.
    pub async fn candidates(&self, day: NaiveDate) -> anyhow::Result<(Vec<Candidate>, DayReport)> {
        let mut report = DayReport::default();
        let Some(index) = self.get(&daily_index_url(day)).await? else {
            // No index: a weekend, a holiday, or a day SEC has not published yet.
            return Ok((Vec::new(), report));
        };
        let entries = parse_form_index(&index);
        report.listed = entries.len();
        let mut out = Vec::new();
        for entry in entries.into_iter().filter(|e| wanted_form(&e.form)) {
            report.wanted += 1;
            let acc = secinfra::format_accession_int(entry.accession, "dash");
            if self.store.is_seen(&acc)? {
                report.already_done += 1;
                continue;
            }
            let page = match self.get(&index_page_url(entry.cik, entry.accession)).await {
                Ok(Some(p)) => p,
                Ok(None) | Err(_) => {
                    report.index_errors += 1;
                    continue;
                }
            };
            match (lists_ex10(&page), accepted_at(&page)) {
                (true, Some(accepted)) => out.push(Candidate { entry, accepted }),
                (true, None) => report.index_errors += 1,
                (false, _) => {
                    self.store.mark_seen(&acc, &entry.form, &entry.cik.to_string())?;
                    report.without_ex10 += 1;
                }
            }
        }
        report.candidates = out.len();
        Ok((out, report))
    }

    /// GET an SEC URL under `sec_base`, paced. Ok(None) on 404; Err on any other
    /// failure.
    async fn get(&self, url: &str) -> anyhow::Result<Option<String>> {
        let url = url.replacen("https://www.sec.gov", &self.sec_base, 1);
        self.pacer.wait().await;
        let resp = self.client.get(&url).send().await?;
        match resp.status().as_u16() {
            200 => Ok(Some(resp.text().await?)),
            404 => Ok(None),
            s => anyhow::bail!("GET {url}: status {s}"),
        }
    }
}

/// The submission the live pipeline would have built for this filing, with SEC's
/// acceptance time as the detection time.
pub fn submission_for(c: &Candidate) -> secinfra::Submission {
    let d = &c.entry.date_filed;
    secinfra::Submission {
        accession: c.entry.accession,
        submission_type: c.entry.form.clone(),
        ciks: vec![c.entry.cik],
        filing_date: format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]),
        size_bytes: None,
        source: secinfra::SubmissionSource::Efts,
        detected_time: c.accepted,
    }
}

#[derive(Debug, Default)]
pub struct RunTotals {
    pub days: usize,
    pub candidates: usize,
    pub filings_with_rows: usize,
    pub rows: usize,
    pub rows_accepted: usize,
    pub retry_next_run: usize,
}

/// Replay `args.from..=args.to` through the live pipeline and delivery.
pub async fn run(cfg: &crate::config::Config, args: &ReplayArgs) -> anyhow::Result<RunTotals> {
    use crate::cooldown::Cooldown;
    use crate::delivery::{Delivery, PUSH_BACKOFF};
    use std::sync::atomic::AtomicU64;

    let client = reqwest::Client::builder()
        .user_agent(&cfg.user_agent)
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let pacer = Pacer::new(args.rps);
    let store = crate::store::Store::open(&args.state)?;
    store.init()?;
    let outbox = crate::outbox::Outbox::open(&cfg.outbox_path)?;
    let cooldown = Cooldown::new(std::time::Duration::from_secs(cfg.push_cooldown_secs));
    let ids = AtomicU64::new(0);
    let lister = Lister { client: &client, sec_base: "https://www.sec.gov".into(), pacer: &pacer, store: &store };
    let delivery = Delivery { client: &client, cfg, outbox: &outbox, cooldown: &cooldown, backoff: PUSH_BACKOFF };
    let mut t = RunTotals::default();

    for day in weekdays(args.from, args.to) {
        let (cands, r) = match lister.candidates(day).await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("replay {day}: day index failed, rerun later: {e:#}");
                continue;
            }
        };
        t.days += 1;
        t.candidates += cands.len();
        tracing::info!(%day, listed = r.listed, wanted = r.wanted, done = r.already_done,
            without_ex10 = r.without_ex10, index_errors = r.index_errors, candidates = r.candidates, "replay day");
        if args.dry_run {
            continue;
        }
        for c in &cands {
            let acc = secinfra::format_accession_int(c.entry.accession, "dash");
            let sub = submission_for(c);
            pacer.wait().await;
            let label = format!("replay {acc}");
            let processed = crate::preflight::isolate(
                &label,
                crate::pipeline::process_submission(&client, cfg, &ids, &sub),
            )
            .await;
            let Some(p) = processed.filter(|p| !p.ex10.is_empty()) else {
                // Fetch failure or nothing extracted: leave it for the next run.
                t.retry_next_run += 1;
                continue;
            };
            t.filings_with_rows += 1;
            t.rows += p.ex10.len();
            t.rows_accepted += delivery.deliver(&p.ex10).await;
            // Rows not accepted are in the outbox, so the filing is done either way.
            store.mark_seen(&acc, &c.entry.form, &c.entry.cik.to_string())?;
        }
    }
    let drained = delivery.drain_once().await;
    tracing::info!(?t, outbox_delivered = drained.delivered, outbox_kept = drained.kept, "replay finished");
    Ok(t)
}

/// Parse `form.YYYYMMDD.idx`. Lines that do not parse (the header) are skipped.
pub fn parse_form_index(text: &str) -> Vec<IndexEntry> {
    text.lines()
        .filter_map(|line| {
            let c = INDEX_LINE.captures(line)?;
            Some(IndexEntry {
                form: c["form"].trim().to_string(),
                company: c["company"].trim().to_string(),
                cik: c["cik"].parse().ok()?,
                date_filed: c["date"].to_string(),
                accession: c["acc"].replace('-', "").parse().ok()?,
            })
        })
        .collect()
}

/// Form types that carry EX-10 exhibits, and their amendments.
pub fn wanted_form(form: &str) -> bool {
    let base = form.trim().strip_suffix("/A").unwrap_or(form.trim());
    WANTED.contains(&base)
}

/// True when the filing index lists an EX-10 document (EX-10, EX-10.1, EX-10.01…),
/// not counting XBRL EX-101.* files.
pub fn lists_ex10(index_html: &str) -> bool {
    EX10_CELL.is_match(index_html)
}

/// SEC's acceptance time from the filing index ("Accepted" is New York time).
pub fn accepted_at(index_html: &str) -> Option<DateTime<Utc>> {
    let raw = ACCEPTED.captures(index_html)?.get(1)?.as_str();
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S").ok()?;
    chrono_tz::America::New_York
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

pub fn daily_index_url(date: NaiveDate) -> String {
    let qtr = (date.month() - 1) / 3 + 1;
    format!(
        "https://www.sec.gov/Archives/edgar/daily-index/{}/QTR{qtr}/form.{}.idx",
        date.year(),
        date.format("%Y%m%d")
    )
}

pub fn index_page_url(cik: u64, accession: u64) -> String {
    format!(
        "https://www.sec.gov/Archives/edgar/data/{cik}/{}/{}-index.htm",
        secinfra::format_accession_int(accession, "nodash"),
        secinfra::format_accession_int(accession, "dash")
    )
}

/// Weekdays from `from` to `to`, inclusive. Holidays have no index (404) and are
/// skipped at fetch time.
pub fn weekdays(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    from.iter_days()
        .take_while(|d| *d <= to)
        .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun))
        .collect()
}

pub fn parse_replay_args(argv: &[String]) -> Result<ReplayArgs, String> {
    let date = |v: Option<&String>, flag: &str| -> Result<NaiveDate, String> {
        let v = v.ok_or_else(|| format!("{flag} requires YYYY-MM-DD"))?;
        NaiveDate::parse_from_str(v, "%Y-%m-%d").map_err(|e| format!("{flag} {v:?}: {e}"))
    };
    let (mut from, mut to, mut rps, mut dry_run, mut state) = (None, None, 2.5_f64, false, "/data/replay.db".to_string());
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--from" => { i += 1; from = Some(date(argv.get(i), "--from")?); }
            "--to" => { i += 1; to = Some(date(argv.get(i), "--to")?); }
            "--rps" => {
                i += 1;
                rps = argv.get(i).and_then(|v| v.parse::<f64>().ok()).filter(|r| *r > 0.0)
                    .ok_or("--rps requires a positive number")?;
            }
            "--state" => { i += 1; state = argv.get(i).ok_or("--state requires a path")?.clone(); }
            "--dry-run" => dry_run = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok(ReplayArgs {
        from: from.ok_or("--from is required")?,
        to: to.ok_or("--to is required")?,
        // The live producer shares SEC's 10 requests/s with the replay.
        rps: rps.min(5.0),
        dry_run,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORM_IDX: &str = include_str!("../tests/fixtures/replay/form.20260810.idx");
    const EX10: &str = include_str!("../tests/fixtures/replay/index-ex10.htm");
    const EX99: &str = include_str!("../tests/fixtures/replay/index-ex99-only.htm");
    const XBRL: &str = include_str!("../tests/fixtures/replay/index-xbrl-ex101-only.htm");

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn parses_the_daily_form_index() {
        let rows = parse_form_index(FORM_IDX);
        assert_eq!(rows.len(), 19, "one entry per edgar/data line; header lines are skipped");
        let aaon = rows.iter().find(|r| r.form == "10-Q" && r.cik == 824142).unwrap();
        assert_eq!(aaon.company, "AAON, INC.");
        assert_eq!(aaon.date_filed, "20260810");
        // Multi-word form types keep their spaces.
        assert!(rows.iter().any(|r| r.form == "SCHEDULE 13G"));
        // Accession from edgar/data/<cik>/0001234567-26-000123.txt
        for r in &rows {
            assert!(r.accession > 0);
        }
    }

    #[test]
    fn keeps_the_forms_that_carry_material_contracts() {
        for f in ["8-K", "8-K/A", "10-Q", "10-Q/A", "10-K", "10-K/A", "S-1", "S-1/A", "S-4", "10-12B", "F-1", "20-F"] {
            assert!(wanted_form(f), "{f}");
        }
        for f in ["4", "424B2", "SCHEDULE 13G", "13F-HR", "144", "D", "6-K", "N-PX"] {
            assert!(!wanted_form(f), "{f}");
        }
    }

    #[test]
    fn detects_ex10_but_not_xbrl_ex101() {
        assert!(lists_ex10(EX10));
        assert!(!lists_ex10(EX99));
        assert!(!lists_ex10(XBRL), "EX-101.SCH is XBRL, not a material contract");
    }

    #[test]
    fn reads_acceptance_time_as_new_york_time() {
        // 2026-08-10 06:02:41 in New York (EDT, UTC-4) is 10:02:41 UTC.
        let t = accepted_at(EX99).unwrap();
        assert_eq!(t.to_rfc3339(), "2026-08-10T10:02:41+00:00");
        assert!(accepted_at("<html>no acceptance</html>").is_none());
    }

    #[test]
    fn builds_sec_urls() {
        assert_eq!(
            daily_index_url(d("2026-08-10")),
            "https://www.sec.gov/Archives/edgar/daily-index/2026/QTR3/form.20260810.idx"
        );
        assert_eq!(
            daily_index_url(d("2026-10-01")),
            "https://www.sec.gov/Archives/edgar/daily-index/2026/QTR4/form.20261001.idx"
        );
        assert_eq!(
            index_page_url(824142, 82414226000052),
            "https://www.sec.gov/Archives/edgar/data/824142/000082414226000052/0000824142-26-000052-index.htm"
        );
    }

    #[test]
    fn walks_weekdays_only() {
        // Fri 2026-08-07 .. Tue 2026-08-11 → Fri, Mon, Tue.
        let days = weekdays(d("2026-08-07"), d("2026-08-11"));
        assert_eq!(days, vec![d("2026-08-07"), d("2026-08-10"), d("2026-08-11")]);
        assert!(weekdays(d("2026-08-11"), d("2026-08-07")).is_empty());
    }

    #[test]
    fn parses_args_with_defaults_and_bounds() {
        let argv = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let a = parse_replay_args(&argv(&["--from", "2026-08-07", "--to", "2026-10-01"])).unwrap();
        assert_eq!((a.from, a.to), (d("2026-08-07"), d("2026-10-01")));
        assert_eq!(a.rps, 2.5);
        assert!(!a.dry_run);
        assert_eq!(a.state, "/data/replay.db");
        let b = parse_replay_args(&argv(&["--from", "2026-08-07", "--to", "2026-08-07", "--rps", "50", "--dry-run", "--state", "/tmp/r.db"])).unwrap();
        assert_eq!(b.rps, 5.0, "capped: the live producer shares SEC's 10/s");
        assert!(b.dry_run);
        assert_eq!(b.state, "/tmp/r.db");
        assert!(parse_replay_args(&argv(&["--from", "2026-08-07"])).is_err());
        assert!(parse_replay_args(&argv(&["--from", "bad", "--to", "2026-08-07"])).is_err());
        assert!(parse_replay_args(&argv(&["--from", "2026-08-07", "--to", "2026-08-07", "--bogus"])).is_err());
    }

    #[test]
    fn builds_the_submission_the_live_pipeline_expects() {
        let entry = parse_form_index(FORM_IDX).into_iter().find(|e| e.form == "8-K").unwrap();
        let accepted = accepted_at(EX10).unwrap();
        let sub = submission_for(&Candidate { entry: entry.clone(), accepted });
        assert_eq!(sub.accession, entry.accession);
        assert_eq!(sub.ciks, vec![entry.cik]);
        assert_eq!(sub.submission_type, "8-K");
        assert_eq!(sub.filing_date, "2026-08-10");
        assert_eq!(sub.detected_time, accepted, "replayed rows carry SEC's acceptance time");
    }

    mod listing {
        use super::*;
        use axum::{Router, extract::State, http::{StatusCode, Uri}};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        type Pages = Arc<Mutex<(HashMap<String, (u16, String)>, Vec<String>)>>;

        async fn serve(State(p): State<Pages>, uri: Uri) -> (StatusCode, String) {
            let mut g = p.lock().unwrap();
            g.1.push(uri.path().to_string());
            match g.0.get(uri.path()) {
                Some((s, body)) => (StatusCode::from_u16(*s).unwrap(), body.clone()),
                None => (StatusCode::NOT_FOUND, String::new()),
            }
        }

        async fn fake_sec(pages: HashMap<String, (u16, String)>) -> (String, Pages) {
            let shared: Pages = Arc::new(Mutex::new((pages, Vec::new())));
            let app = Router::new().fallback(serve).with_state(shared.clone());
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = l.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
            (format!("http://{addr}"), shared)
        }

        fn path_of(url: &str) -> String {
            url.trim_start_matches("https://www.sec.gov").to_string()
        }

        fn store() -> crate::store::Store {
            let s = crate::store::Store::open(":memory:").unwrap();
            s.init().unwrap();
            s
        }

        #[tokio::test]
        async fn lists_only_wanted_filings_with_an_ex10() {
            let day = d("2026-08-10");
            let entries = parse_form_index(FORM_IDX);
            let wanted: Vec<&IndexEntry> = entries.iter().filter(|e| wanted_form(&e.form)).collect();
            assert_eq!(wanted.len(), 10);
            let mut pages = HashMap::new();
            pages.insert(path_of(&daily_index_url(day)), (200, FORM_IDX.to_string()));
            for (i, e) in wanted.iter().enumerate() {
                let body = match i { 0 => (200, EX10.to_string()), 1 => (500, String::new()), _ => (200, EX99.to_string()) };
                pages.insert(path_of(&index_page_url(e.cik, e.accession)), body);
            }
            let (base, seen) = fake_sec(pages).await;
            let (client, pacer, st) = (reqwest::Client::new(), Pacer::new(1000.0), store());
            let lister = Lister { client: &client, sec_base: base, pacer: &pacer, store: &st };

            let (c, r) = lister.candidates(day).await.unwrap();
            assert_eq!(c.len(), 1);
            assert_eq!(c[0].entry.accession, wanted[0].accession);
            assert_eq!(c[0].accepted.to_rfc3339(), accepted_at(EX10).unwrap().to_rfc3339());
            assert_eq!(r, DayReport { listed: 19, wanted: 10, already_done: 0, without_ex10: 8, index_errors: 1, candidates: 1 });
            // Unwanted forms are never fetched: 1 day index + 10 filing indexes.
            assert_eq!(seen.lock().unwrap().1.len(), 11);

            // Second pass: the 8 without an EX-10 are done; the error and the candidate are retried.
            let (c2, r2) = lister.candidates(day).await.unwrap();
            assert_eq!(c2.len(), 1);
            assert_eq!(r2.already_done, 8);
            assert_eq!(seen.lock().unwrap().1.len(), 11 + 1 + 2);
        }

        #[tokio::test]
        async fn a_day_without_an_index_is_empty_not_an_error() {
            let (base, _) = fake_sec(HashMap::new()).await;
            let (client, pacer, st) = (reqwest::Client::new(), Pacer::new(1000.0), store());
            let lister = Lister { client: &client, sec_base: base, pacer: &pacer, store: &st };
            let (c, r) = lister.candidates(d("2026-09-07")).await.unwrap(); // Labor Day
            assert!(c.is_empty());
            assert_eq!(r, DayReport::default());
        }

        #[tokio::test]
        async fn a_failing_day_index_is_an_error() {
            let day = d("2026-08-10");
            let mut pages = HashMap::new();
            pages.insert(path_of(&daily_index_url(day)), (503, String::new()));
            let (base, _) = fake_sec(pages).await;
            let (client, pacer, st) = (reqwest::Client::new(), Pacer::new(1000.0), store());
            let lister = Lister { client: &client, sec_base: base, pacer: &pacer, store: &st };
            assert!(lister.candidates(day).await.is_err());
        }
    }
}
