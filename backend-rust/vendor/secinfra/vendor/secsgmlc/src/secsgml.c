#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stddef.h>

#include "secsgml.h"
#include "uudecode.h"

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
static const char DOC_OPEN[]   = "<DOCUMENT>";
static const char DOC_CLOSE[]  = "</DOCUMENT>";
static const char TEXT_OPEN[]  = "<TEXT>";
static const char TEXT_CLOSE[] = "</TEXT>";
static const char BEGIN_644[]  = "begin 644";

#define DOC_OPEN_LEN   (sizeof(DOC_OPEN)   - 1)
#define DOC_CLOSE_LEN  (sizeof(DOC_CLOSE)  - 1)
#define TEXT_OPEN_LEN  (sizeof(TEXT_OPEN)  - 1)
#define TEXT_CLOSE_LEN (sizeof(TEXT_CLOSE) - 1)
#define BEGIN_644_LEN  (sizeof(BEGIN_644)  - 1)

#define UU_DECODE_MAX  (256u * 1024u * 1024u)

// Tag suffixes after '<' (first char already consumed)
// We dispatch on buf[1] after finding '<'
#define SUFFIX_DOCUMENT    "DOCUMENT>"        // <DOCUMENT>
#define SUFFIX_EDOCUMENT   "/DOCUMENT>"       // </DOCUMENT>
#define SUFFIX_TEXT        "TEXT>"            // <TEXT>
#define SUFFIX_ETEXT       "/TEXT>"           // </TEXT>
#define SUFFIX_TYPE        "TYPE>"            // <TYPE>
#define SUFFIX_SEQUENCE    "SEQUENCE>"        // <SEQUENCE>
#define SUFFIX_FILENAME    "FILENAME>"        // <FILENAME>
#define SUFFIX_DESCRIPTION "DESCRIPTION>"     // <DESCRIPTION>

#define DOCS_INITIAL_CAP   64
#define EVENTS_INITIAL_CAP 128

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------
static inline int is_space(uint8_t c) {
    return c == ' ' || c == '\t' || c == '\r' || c == '\n';
}

static inline const uint8_t *find_eol(const uint8_t *p, const uint8_t *end) {
    while (p < end && *p != '\n' && *p != '\r') p++;
    return p;
}

static inline const uint8_t *skip_eol(const uint8_t *p, const uint8_t *end) {
    if (p < end && *p == '\r') p++;
    if (p < end && *p == '\n') p++;
    return p;
}

static inline byte_span trim_span(byte_span s) {
    while (s.len > 0 && is_space(s.ptr[0]))         { s.ptr++; s.len--; }
    while (s.len > 0 && is_space(s.ptr[s.len - 1])) { s.len--; }
    return s;
}

static inline byte_span ltrim_span(byte_span s) {
    while (s.len > 0 && is_space(s.ptr[0])) { s.ptr++; s.len--; }
    return s;
}

// Used only for submission metadata and find_uu_bounds (not hot path)
static const uint8_t *find_subspan(const uint8_t *hay, size_t hay_len,
                                    const char *needle, size_t needle_len) {
    if (needle_len == 0 || hay_len < needle_len) return NULL;
    uint8_t first = (uint8_t)needle[0];
    const uint8_t *end = hay + hay_len - needle_len;
    for (const uint8_t *p = hay; p <= end; p++) {
        p = (const uint8_t *)memchr(p, first, (size_t)(end - p) + 1);
        if (!p) return NULL;
        if (memcmp(p, needle, needle_len) == 0) return p;
    }
    return NULL;
}

static size_t find_double_newline(const uint8_t *buf, size_t len) {
    for (size_t i = 0; i + 1 < len; i++) {
        if (buf[i] == '\n' && buf[i+1] == '\n') return i + 2;
        if (buf[i] == '\r' && i+3 < len &&
            buf[i+1] == '\n' && buf[i+2] == '\r' && buf[i+3] == '\n') return i + 4;
    }
    return len;
}

// Read value from current position to end of line, trimmed
static inline byte_span value_to_eol(const uint8_t *p, const uint8_t *end) {
    const uint8_t *eol = find_eol(p, end);
    return trim_span((byte_span){ p, (size_t)(eol - p) });
}

// ---------------------------------------------------------------------------
// Document array helpers
// ---------------------------------------------------------------------------
static int docs_push(sgml_parse_result *r, document doc) {
    if (r->doc_count == r->doc_cap) {
        size_t new_cap = r->doc_cap ? r->doc_cap * 2 : DOCS_INITIAL_CAP;
        document *tmp = (document *)realloc(r->docs, new_cap * sizeof(document));
        if (!tmp) return 0;
        r->docs    = tmp;
        r->doc_cap = new_cap;
    }
    r->docs[r->doc_count++] = doc;
    return 1;
}

// ---------------------------------------------------------------------------
// UU detection -- only called once we know we're in a <TEXT> block
// ---------------------------------------------------------------------------
static inline int is_begin_644(const uint8_t *p, size_t len) {
    if (len < BEGIN_644_LEN) return 0;
    if (memcmp(p, BEGIN_644, BEGIN_644_LEN) != 0) return 0;
    return len == BEGIN_644_LEN || is_space(p[BEGIN_644_LEN]);
}

static inline int is_end_line(const uint8_t *p, size_t len) {
    if (len < 3 || memcmp(p, "end", 3) != 0) return 0;
    return len == 3 || is_space(p[3]);
}

static int find_uu_bounds(const uint8_t *text_start, const uint8_t *text_end,
                           const uint8_t **enc_start, const uint8_t **enc_end) {
    const uint8_t *p = text_start;
    for (int line = 0; line < 3 && p < text_end; line++) {
        const uint8_t *eol = find_eol(p, text_end);
        if (is_begin_644(p, (size_t)(eol - p))) {
            const uint8_t *enc = skip_eol(eol, text_end);
            const uint8_t *scan = enc;
            const uint8_t *enc_e = text_end;
            while (scan < text_end) {
                const uint8_t *le = find_eol(scan, text_end);
                if (is_end_line(scan, (size_t)(le - scan))) {
                    enc_e = scan;
                    break;
                }
                scan = skip_eol(le, text_end);
            }
            *enc_start = enc;
            *enc_end   = enc_e;
            return 1;
        }
        p = skip_eol(eol, text_end);
    }
    return 0;
}

// Pre-scan uuencoded payload to compute exact decoded size.
// Uses the same length-char rules as uudecode, so malformed lines are still counted safely.
static size_t uu_decoded_size(const uint8_t *enc_start, const uint8_t *enc_end, int *capped) {
    size_t total = 0;
    const uint8_t *p = enc_start;
    if (capped) *capped = 0;
    while (p < enc_end) {
        const uint8_t *eol = find_eol(p, enc_end);
        if (p < eol) {
            uint8_t len_char = *p;
            int nbytes = (len_char - 32) & 0x3f;
            if (nbytes == 0) break;
            if (total > UU_DECODE_MAX - (size_t)nbytes) {
                total = UU_DECODE_MAX;
                if (capped) *capped = 1;
                break;
            }
            total += (size_t)nbytes;
        }
        p = skip_eol(eol, enc_end);
    }
    return total;
}

static void strip_wrappers(const uint8_t **start, const uint8_t **end) {
    const uint8_t *s = *start;
    const uint8_t *e = *end;
    while (s < e && is_space(*s)) s++;
    while (e > s && is_space(*(e-1))) e--;

    struct { const char *open; size_t olen; const char *close; size_t clen; } tags[] = {
        { "<PDF>",  5, "</PDF>",  6 },
        { "<XBRL>", 6, "</XBRL>", 7 },
        { "<XML>",  5, "</XML>",  6 },
    };
    for (int i = 0; i < 3; i++) {
        if ((size_t)(e - s) >= tags[i].olen &&
            memcmp(s, tags[i].open, tags[i].olen) == 0) {
            s += tags[i].olen;
            while (s < e && is_space(*s)) s++;
        }
        if ((size_t)(e - s) >= tags[i].clen &&
            memcmp(e - tags[i].clen, tags[i].close, tags[i].clen) == 0) {
            e -= tags[i].clen;
            while (e > s && is_space(*(e-1))) e--;
        }
    }
    *start = s;
    *end   = e;
}

// ---------------------------------------------------------------------------
// Single-pass scanner state
// ---------------------------------------------------------------------------
typedef enum {
    STATE_BETWEEN,      // between documents
    STATE_IN_DOC_META,  // inside <DOCUMENT>, before <TEXT>
    STATE_IN_TEXT,      // inside <TEXT>...</TEXT>
} scan_state;

// ---------------------------------------------------------------------------
// Main parse_sgml -- single pass over '<' characters
// ---------------------------------------------------------------------------
sgml_parse_result parse_sgml(const uint8_t *buf, size_t len, sgml_parse_stats *stats) {
    sgml_parse_result result = {0};
    result.status = SGML_STATUS_OK;
    result.docs    = (document *)malloc(DOCS_INITIAL_CAP * sizeof(document));
    result.doc_cap = result.docs ? DOCS_INITIAL_CAP : 0;
    if (!result.docs) {
        result.status = SGML_STATUS_OOM;
        return result;
    }

    const uint8_t *p   = buf;
    const uint8_t *end = buf + len;

    scan_state state = STATE_BETWEEN;
    document   cur   = {0};          // document being built
    while (p < end) {

        // Jump to next '<' -- memchr is SIMD-optimized on Linux/glibc
        const uint8_t *lt = (const uint8_t *)memchr(p, '<', (size_t)(end - p));
        if (!lt) { break; }

        // How many bytes remain after '<'
        size_t remain = (size_t)(end - lt);

        // Dispatch on first char after '<'
        // All our tags start with distinct leading chars:
        //   D -> DOCUMENT>
        //   / -> /DOCUMENT> or /TEXT>
        //   T -> TEXT> or TYPE>
        //   S -> SEQUENCE>
        //   F -> FILENAME>
        //   E -> (none, but DESCRIPTION starts with D... wait no)
        //   d -> DESCRIPTION starts with D uppercase
        // Actually: TYPE and TEXT both start with T, DOCUMENT and DESCRIPTION both start with D
        // So we need two chars for disambiguation in those cases.

        if (remain < 2) { break; }

        uint8_t c1 = lt[1];

        if (c1 == 'D') {
            // Could be <DOCUMENT> or <DESCRIPTION>
            if (remain >= 10 && memcmp(lt+1, "DOCUMENT>", 9) == 0) {
                // <DOCUMENT>
                if (state == STATE_BETWEEN) {
                    cur = (document){0};
                    state = STATE_IN_DOC_META;
                }
                p = lt + 10;
            } else if (remain >= 13 && memcmp(lt+1, "DESCRIPTION>", 12) == 0) {
                // <DESCRIPTION>
                if (state == STATE_IN_DOC_META) {
                    cur.meta.description = value_to_eol(lt + 13, end);
                }
                p = lt + 13;
            } else {
                p = lt + 1;
            }

        } else if (c1 == '/') {
            if (remain < 3) { p = lt + 1; continue; }
            uint8_t c2 = lt[2];

            if (c2 == 'D' && remain >= 11 && memcmp(lt+1, "/DOCUMENT>", 10) == 0) {
                // </DOCUMENT>
                if (state == STATE_IN_DOC_META || state == STATE_IN_TEXT) {

                    // UU detect + decode
                    if (state == STATE_IN_TEXT && cur.content_start) {
                        const uint8_t *text_end_ptr = lt; // shouldn't happen but safe fallback
                        // content_start was set at <TEXT>, content_len at </TEXT>
                        // we already set these at </TEXT>, so just finalize
                    }

                    if (stats) stats->doc_count++;
                    if (!docs_push(&result, cur)) {
                        if (cur.decoded) free(cur.decoded);
                        result.status = SGML_STATUS_OOM;
                        return result;
                    }
                    cur   = (document){0};
                    state = STATE_BETWEEN;
                    p = lt + 11;
                    continue;
                }
                p = lt + 11;
            } else if (c2 == 'T' && remain >= 7 && memcmp(lt+1, "/TEXT>", 6) == 0) {
                // </TEXT>
                if (state == STATE_IN_TEXT) {
                    const uint8_t *text_end_ptr = lt;

                    const uint8_t *enc_start, *enc_end;
                    int is_uu = find_uu_bounds(cur.content_start, text_end_ptr, &enc_start, &enc_end);

                    if (is_uu) {
                        cur.is_uuencoded  = 1;
                        cur.content_start = enc_start;
                        cur.content_len   = (size_t)(enc_end - enc_start);
                        int capped = 0;
                        size_t dec_sz = uu_decoded_size(enc_start, enc_end, &capped);
                        if (capped) result.status = SGML_STATUS_TRUNCATED;
                        cur.decoded = (uint8_t *)malloc(dec_sz ? dec_sz : 1);
                        if (cur.decoded) {
                            cur.decoded_len = uudecode(enc_start, cur.content_len, cur.decoded, dec_sz);
                            if (cur.decoded_len != dec_sz)
                                result.status = SGML_STATUS_TRUNCATED;
                        } else {
                            result.status = SGML_STATUS_OOM;
                        }
                        if (stats) stats->uuencoded_count++;
                    } else {
                        cur.is_uuencoded = 0;
                        const uint8_t *cs = cur.content_start;
                        const uint8_t *ce = text_end_ptr;
                        strip_wrappers(&cs, &ce);
                        cur.content_start = cs;
                        cur.content_len   = (size_t)(ce - cs);
                    }

                    state = STATE_IN_DOC_META; // back to meta state until </DOCUMENT>
                    p = lt + 7;
                    continue;
                }
                p = lt + 7;
            } else {
                p = lt + 1;
            }

        } else if (c1 == 'T') {
            // Could be <TEXT> or <TYPE>
            if (remain >= 6 && memcmp(lt+1, "TEXT>", 5) == 0) {
                // <TEXT>
                if (state == STATE_IN_DOC_META) {
                    cur.content_start = lt + 6; // points to byte after <TEXT>
                    state = STATE_IN_TEXT;
                }
                p = lt + 6;
            } else if (remain >= 6 && memcmp(lt+1, "TYPE>", 5) == 0) {
                // <TYPE>
                if (state == STATE_IN_DOC_META) {
                    cur.meta.type = value_to_eol(lt + 6, end);
                }
                p = lt + 6;
            } else {
                p = lt + 1;
            }

        } else if (c1 == 'S' && remain >= 10 && memcmp(lt+1, "SEQUENCE>", 9) == 0) {
            // <SEQUENCE>
            if (state == STATE_IN_DOC_META) {
                cur.meta.sequence = value_to_eol(lt + 10, end);
            }
            p = lt + 10;

        } else if (c1 == 'F' && remain >= 10 && memcmp(lt+1, "FILENAME>", 9) == 0) {
            // <FILENAME>
            if (state == STATE_IN_DOC_META) {
                cur.meta.filename = value_to_eol(lt + 10, end);
            }
            p = lt + 10;

        } else {
            // Not a tag we care about, skip past this '<'
            p = lt + 1;
        }

    }

    return result;
}

void free_sgml_parse_result(sgml_parse_result *r) {
    if (!r) return;
    for (size_t i = 0; i < r->doc_count; i++) {
        if (r->docs[i].decoded) free(r->docs[i].decoded);
    }
    free(r->docs);
    r->docs      = NULL;
    r->doc_count = 0;
    r->doc_cap   = 0;
}

// ---------------------------------------------------------------------------
// Submission metadata (unchanged -- not hot path)
// ---------------------------------------------------------------------------
static int add_event(submission_metadata *m, submission_event_type type,
                     byte_span key, byte_span value, int depth) {
    if (m->count == m->cap) {
        size_t new_cap = m->cap ? m->cap * 2 : EVENTS_INITIAL_CAP;
        submission_event *tmp = (submission_event *)realloc(m->events, new_cap * sizeof(submission_event));
        if (!tmp) return 0;
        m->events = tmp;
        m->cap    = new_cap;
    }
    m->events[m->count].type  = type;
    m->events[m->count].key   = key;
    m->events[m->count].value = value;
    m->events[m->count].depth = depth;
    m->count++;
    return 1;
}

static int parse_archive_metadata(submission_metadata *m, const uint8_t *buf, size_t len) {
    const uint8_t *p   = buf;
    const uint8_t *end = buf + len;
    int depth = 0;

    while (p < end) {
        const uint8_t *eol = find_eol(p, end);
        byte_span line = ltrim_span((byte_span){ p, (size_t)(eol - p) });

        if (line.len > 0 && line.ptr[0] == '<') {
            const uint8_t *gt = (const uint8_t *)memchr(line.ptr, '>', line.len);
            if (gt) {
                byte_span key   = trim_span((byte_span){ line.ptr + 1, (size_t)(gt - line.ptr - 1) });
                byte_span value = trim_span((byte_span){ gt + 1, (size_t)(line.ptr + line.len - gt - 1) });
                if (key.len == 10 && memcmp(key.ptr, "SUBMISSION", 10) == 0) {
                    // Skip SUBMISSION wrapper to mirror legacy output.
                } else if (key.len > 0 && key.ptr[0] == '/') {
                    if (depth > 0) depth--;
                    if (!add_event(m, SUB_EVENT_SECTION_END, key, (byte_span){0}, depth)) return 0;
                } else if (value.len == 0) {
                    if (!add_event(m, SUB_EVENT_SECTION_START, key, (byte_span){0}, depth)) return 0;
                    depth++;
                } else {
                    if (!add_event(m, SUB_EVENT_KEYVAL, key, value, depth)) return 0;
                }
            }
        }
        p = skip_eol(eol, end);
    }
    while (depth-- > 0)
        if (!add_event(m, SUB_EVENT_SECTION_END, (byte_span){0}, (byte_span){0}, depth)) return 0;
    return 1;
}

static int parse_tab_metadata(submission_metadata *m, const uint8_t *buf, size_t len) {
    const uint8_t *p   = buf;
    const uint8_t *end = buf + len;
    int depth = 0;

    while (p < end) {
        const uint8_t *eol = find_eol(p, end);
        byte_span line = { p, (size_t)(eol - p) };
        if (line.len == 0) { p = skip_eol(eol, end); continue; }

        size_t indent = 0;
        while (indent < line.len && line.ptr[indent] == '\t') indent++;
        byte_span content = trim_span((byte_span){ line.ptr + indent, line.len - indent });
        if (content.len == 0) { p = skip_eol(eol, end); continue; }

        while (depth > (int)indent) {
            depth--;
            if (!add_event(m, SUB_EVENT_SECTION_END, (byte_span){0}, (byte_span){0}, depth)) return 0;
        }

        const uint8_t *colon = (const uint8_t *)memchr(content.ptr, ':', content.len);
        if (colon) {
            byte_span key   = trim_span((byte_span){ content.ptr, (size_t)(colon - content.ptr) });
            byte_span value = trim_span((byte_span){ colon + 1, (size_t)(content.ptr + content.len - colon - 1) });
            if (value.len == 0) {
                if (!add_event(m, SUB_EVENT_SECTION_START, key, (byte_span){0}, depth)) return 0;
                depth++;
            } else {
                if (!add_event(m, SUB_EVENT_KEYVAL, key, value, depth)) return 0;
            }
        } else if (content.ptr[0] == '<') {
            const uint8_t *gt = (const uint8_t *)memchr(content.ptr, '>', content.len);
            if (gt) {
                byte_span key   = trim_span((byte_span){ content.ptr + 1, (size_t)(gt - content.ptr - 1) });
                byte_span value = trim_span((byte_span){ gt + 1, (size_t)(content.ptr + content.len - gt - 1) });
                if (key.len > 0 && key.ptr[0] == '/') {
                    if (depth > 0) depth--;
                    if (!add_event(m, SUB_EVENT_SECTION_END, key, (byte_span){0}, depth)) return 0;
                } else if (value.len == 0) {
                    if (!add_event(m, SUB_EVENT_SECTION_START, key, (byte_span){0}, depth)) return 0;
                    depth++;
                } else {
                    if (!add_event(m, SUB_EVENT_KEYVAL, key, value, depth)) return 0;
                }
            }
        }
        p = skip_eol(eol, end);
    }
    while (depth-- > 0)
        if (!add_event(m, SUB_EVENT_SECTION_END, (byte_span){0}, (byte_span){0}, depth)) return 0;
    return 1;
}

submission_metadata parse_submission_metadata(const uint8_t *buf, size_t len) {
    submission_metadata m = {0};
    m.status = SGML_STATUS_OK;

    const uint8_t *doc_start = find_subspan(buf, len, DOC_OPEN, DOC_OPEN_LEN);
    size_t sub_len = doc_start ? (size_t)(doc_start - buf) : len;
    if (sub_len == 0) return m;

    byte_span sub = ltrim_span((byte_span){ buf, sub_len });
    if (sub.len == 0) return m;

    if (sub.ptr[0] == '-') {
        size_t privacy_end = find_double_newline(sub.ptr, sub.len);
        if (privacy_end > 0 && privacy_end < sub.len) {
            byte_span key   = { (const uint8_t *)"PRIVACY-ENHANCED-MESSAGE", 24 };
            byte_span value = { sub.ptr, privacy_end };
            if (!add_event(&m, SUB_EVENT_KEYVAL, key, value, 0)) {
                m.status = SGML_STATUS_OOM;
                return m;
            }
            sub.ptr += privacy_end;
            sub.len -= privacy_end;
            sub = ltrim_span(sub);
        }
        if (!parse_tab_metadata(&m, sub.ptr, sub.len)) {
            m.status = SGML_STATUS_OOM;
            return m;
        }
    } else if (sub.len >= 3 && memcmp(sub.ptr, "<SE", 3) == 0) {
        if (!parse_tab_metadata(&m, sub.ptr, sub.len)) {
            m.status = SGML_STATUS_OOM;
            return m;
        }
    } else {
        if (!parse_archive_metadata(&m, sub.ptr, sub.len)) {
            m.status = SGML_STATUS_OOM;
            return m;
        }
    }

    return m;
}

void free_submission_metadata(submission_metadata *m) {
    if (!m) return;
    free(m->events);
    m->events = NULL;
    m->count  = 0;
    m->cap    = 0;
}
