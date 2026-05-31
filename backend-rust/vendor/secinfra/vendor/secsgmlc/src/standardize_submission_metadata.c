#include "standardize_submission_metadata.h"

#include <stdlib.h>
#include <string.h>

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------
static inline int is_space(uint8_t c) {
    return c == ' ' || c == '\t' || c == '\r' || c == '\n';
}

static inline int is_digit(uint8_t c) {
    return c >= '0' && c <= '9';
}

static inline uint8_t to_lower_ascii(uint8_t c) {
    if (c >= 'A' && c <= 'Z') return (uint8_t)(c + 32);
    return c;
}

static int arena_ensure(standardized_submission_metadata *out, size_t extra) {
    if (out->arena_len + extra <= out->arena_cap) return 1;
    size_t new_cap = out->arena_cap ? out->arena_cap * 2 : 1024;
    while (new_cap < out->arena_len + extra) new_cap *= 2;
    uint8_t *tmp = (uint8_t *)realloc(out->arena, new_cap);
    if (!tmp) return 0;
    out->arena = tmp;
    out->arena_cap = new_cap;
    return 1;
}

static byte_span arena_append(standardized_submission_metadata *out,
                              const uint8_t *src, size_t len) {
    if (len == 0 || !src) return (byte_span){0};
    if (!arena_ensure(out, len)) return (byte_span){0};
    uint8_t *dst = out->arena + out->arena_len;
    memcpy(dst, src, len);
    out->arena_len += len;
    return (byte_span){ dst, len };
}

static int events_ensure(standardized_submission_metadata *out, size_t extra) {
    if (out->count + extra <= out->cap) return 1;
    size_t new_cap = out->cap ? out->cap * 2 : 128;
    while (new_cap < out->count + extra) new_cap *= 2;
    submission_event *tmp = (submission_event *)realloc(out->events, new_cap * sizeof(submission_event));
    if (!tmp) return 0;
    out->events = tmp;
    out->cap = new_cap;
    return 1;
}

// ---------------------------------------------------------------------------
// Mapping table
// ---------------------------------------------------------------------------
typedef enum {
    REGEX_NONE = 0,
    REGEX_SEC_ACT,
    REGEX_SIC
} regex_kind;

typedef struct {
    const char *from;
    const char *to;
    regex_kind  rx;
} map_entry;

static const map_entry MAP[] = {
    { "paper", "paper", REGEX_NONE },
    { "accession number", "accession-number", REGEX_NONE },
    { "conformed submission type", "type", REGEX_NONE },
    { "public document count", "public-document-count", REGEX_NONE },
    { "public document_count", "public-document-count", REGEX_NONE },
    { "conformed period of report", "period", REGEX_NONE },
    { "filed as of date", "filing-date", REGEX_NONE },
    { "date as of change", "date-of-filing-date-change", REGEX_NONE },
    { "effectiveness date", "effectiveness-date", REGEX_NONE },
    { "filer", "filer", REGEX_NONE },
    { "company data", "company-data", REGEX_NONE },
    { "company conformed name", "conformed-name", REGEX_NONE },
    { "central index key", "cik", REGEX_NONE },
    { "state of incorporation", "state-of-incorporation", REGEX_NONE },
    { "fiscal year end", "fiscal-year-end", REGEX_NONE },
    { "filing values", "filing-values", REGEX_NONE },
    { "form type", "form-type", REGEX_NONE },
    { "sec act", "act", REGEX_SEC_ACT },
    { "sec file number", "file-number", REGEX_NONE },
    { "film number", "film-number", REGEX_NONE },
    { "business address", "business-address", REGEX_NONE },
    { "street 1", "street1", REGEX_NONE },
    { "city", "city", REGEX_NONE },
    { "state", "state", REGEX_NONE },
    { "zip", "zip", REGEX_NONE },
    { "business phone", "phone", REGEX_NONE },
    { "mail address", "mail-address", REGEX_NONE },
    { "former company", "former-company", REGEX_NONE },
    { "former conformed name", "former-conformed-name", REGEX_NONE },
    { "date of name change", "date-changed", REGEX_NONE },
    { "sros", "sros", REGEX_NONE },
    { "subject company", "subject-company", REGEX_NONE },
    { "standard industrial classification", "assigned-sic", REGEX_SIC },
    { "irs number", "irs-number", REGEX_NONE },
    { "filed by", "filed-by", REGEX_NONE },
    { "street 2", "street2", REGEX_NONE },
    { "items", "items", REGEX_NONE },
    { "group members", "group-members", REGEX_NONE },
    { "organization name", "organization-name", REGEX_NONE },
    { "recieved date", "recieved-date", REGEX_NONE },
    { "action date", "action-date", REGEX_NONE },
    { "non us state territory", "non-us-state-territory", REGEX_NONE },
    { "address is a non us location", "address-is-a-non-us-location", REGEX_NONE },
    { "ein", "ein", REGEX_NONE },
    { "class-contract-ticker-symbol", "class-contract-ticker-symbol", REGEX_NONE },
    { "class-contract-name", "class-contract-name", REGEX_NONE },
    { "class-contract-id", "class-contract-id", REGEX_NONE },
    { "sec-document", "sec-document", REGEX_NONE },
    { "sec-header", "sec-header", REGEX_NONE },
    { "acceptance-datetime", "acceptance-datetime", REGEX_NONE },
    { "series-and-classes-contracts-data", "series-and-classes-contracts-data", REGEX_NONE },
    { "existing-series-and-classes-contracts", "existing-series-and-classes-contracts", REGEX_NONE },
    { "merger-series-and-classes-contracts", "merger-series-and-classes-contracts", REGEX_NONE },
    { "new-series-and-classes-contracts", "new-series-and-classes-contracts", REGEX_NONE },
    { "series", "series", REGEX_NONE },
    { "owner-cik", "owner-cik", REGEX_NONE },
    { "series-id", "series-id", REGEX_NONE },
    { "series-name", "series-name", REGEX_NONE },
    { "acquiring-data", "acquiring-data", REGEX_NONE },
    { "target-data", "target-data", REGEX_NONE },
    { "new-classes-contracts", "new-classes-contracts", REGEX_NONE },
    { "new-series", "new-series", REGEX_NONE },
    { "relationship", "relationship", REGEX_NONE }
};

static const map_entry *lookup_map(const uint8_t *key, size_t len) {
    if (!key || len == 0) return NULL;
    for (size_t i = 0; i < sizeof(MAP) / sizeof(MAP[0]); i++) {
        const char *from = MAP[i].from;
        size_t flen = strlen(from);
        if (flen != len) continue;
        if (memcmp(key, from, len) == 0) return &MAP[i];
    }
    return NULL;
}

// ---------------------------------------------------------------------------
// Regex-like extractors (minimal, matches Python rules)
// ---------------------------------------------------------------------------
static int extract_sec_act(byte_span value, byte_span *out) {
    if (!value.ptr || value.len < 7) return 0; // "1933 Act" is 8 incl space
    for (size_t i = 0; i + 6 < value.len; i++) {
        if (!is_digit(value.ptr[i]) ||
            !is_digit(value.ptr[i + 1]) ||
            !is_digit(value.ptr[i + 2]) ||
            !is_digit(value.ptr[i + 3])) {
            continue;
        }
        size_t j = i + 4;
        if (!is_space(value.ptr[j])) continue;
        while (j < value.len && is_space(value.ptr[j])) j++;
        if (j + 2 < value.len &&
            value.ptr[j] == 'A' &&
            value.ptr[j + 1] == 'c' &&
            value.ptr[j + 2] == 't') {
            *out = (byte_span){ value.ptr + i + 2, 2 };
            return 1;
        }
    }
    return 0;
}

static int extract_sic(byte_span value, byte_span *out) {
    if (!value.ptr || value.len < 3) return 0;
    for (size_t i = 0; i + 2 < value.len; i++) {
        if (value.ptr[i] != '[') continue;
        size_t j = i + 1;
        if (!is_digit(value.ptr[j])) continue;
        while (j < value.len && is_digit(value.ptr[j])) j++;
        if (j < value.len && value.ptr[j] == ']') {
            *out = (byte_span){ value.ptr + i + 1, (size_t)(j - (i + 1)) };
            return 1;
        }
    }
    return 0;
}

// Build a lowercase key for lookup (ASCII only).
static uint8_t *build_lower_key(const uint8_t *src, size_t len, uint8_t *stack_buf, size_t stack_cap) {
    uint8_t *buf = stack_buf;
    if (len > stack_cap) {
        buf = (uint8_t *)malloc(len);
        if (!buf) return NULL;
    }
    for (size_t i = 0; i < len; i++) {
        buf[i] = to_lower_ascii(src[i]);
    }
    return buf;
}

static void free_lower_key(uint8_t *buf, uint8_t *stack_buf) {
    if (buf && buf != stack_buf) free(buf);
}

// Build fallback key: lowercase and replace runs of whitespace with '-'
static uint8_t *build_fallback_key(const uint8_t *src, size_t len, size_t *out_len) {
    if (!out_len) return NULL;
    if (!src || len == 0) { *out_len = 0; return NULL; }
    uint8_t *buf = (uint8_t *)malloc(len ? len : 1);
    if (!buf) { *out_len = 0; return NULL; }
    size_t w = 0;
    int in_ws = 0;
    for (size_t i = 0; i < len; i++) {
        uint8_t c = to_lower_ascii(src[i]);
        if (is_space(c)) {
            if (!in_ws) {
                buf[w++] = '-';
                in_ws = 1;
            }
        } else {
            buf[w++] = c;
            in_ws = 0;
        }
    }
    *out_len = w;
    return buf;
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------
standardized_submission_metadata standardize_submission_metadata(const submission_metadata *m) {
    standardized_submission_metadata out;
    memset(&out, 0, sizeof(out));
    out.status = SGML_STATUS_OK;
    if (!m || m->count == 0) return out;
    if (m->status != SGML_STATUS_OK) {
        out.status = m->status;
        return out;
    }

    if (!events_ensure(&out, m->count)) {
        out.status = SGML_STATUS_OOM;
        return out;
    }

    for (size_t i = 0; i < m->count; i++) {
        submission_event ev = m->events[i];
        submission_event new_ev;
        memset(&new_ev, 0, sizeof(new_ev));
        new_ev.type = ev.type;
        new_ev.depth = ev.depth;

        // Normalize key
        byte_span key_in = ev.key;
        int has_slash = 0;
        const uint8_t *kptr = key_in.ptr;
        size_t klen = key_in.len;
        if (klen > 0 && kptr && kptr[0] == '/') {
            has_slash = 1;
            kptr++;
            klen--;
        }

        byte_span key_out = {0};
        if (klen > 0 && kptr) {
            uint8_t stack_buf[256];
            uint8_t *lower = build_lower_key(kptr, klen, stack_buf, sizeof(stack_buf));
            if (lower) {
                const map_entry *me = lookup_map(lower, klen);
                if (me) {
                    size_t out_len = strlen(me->to);
                    if (has_slash) {
                        size_t total = out_len + 1;
                        uint8_t *tmp = (uint8_t *)malloc(total);
                        if (tmp) {
                            tmp[0] = '/';
                            memcpy(tmp + 1, me->to, out_len);
                            key_out = arena_append(&out, tmp, total);
                            free(tmp);
                            if (key_out.ptr == NULL && key_out.len == 0) {
                                out.status = SGML_STATUS_OOM;
                                free_lower_key(lower, stack_buf);
                                break;
                            }
                        } else {
                            out.status = SGML_STATUS_OOM;
                            free_lower_key(lower, stack_buf);
                            break;
                        }
                    } else {
                        key_out = arena_append(&out, (const uint8_t *)me->to, out_len);
                        if (key_out.ptr == NULL && key_out.len == 0) {
                            out.status = SGML_STATUS_OOM;
                            free_lower_key(lower, stack_buf);
                            break;
                        }
                    }
                } else {
                    size_t fallback_len = 0;
                    uint8_t *fallback = build_fallback_key(kptr, klen, &fallback_len);
                    if (fallback) {
                        if (has_slash) {
                            size_t total = fallback_len + 1;
                            uint8_t *tmp = (uint8_t *)malloc(total);
                            if (tmp) {
                                tmp[0] = '/';
                                memcpy(tmp + 1, fallback, fallback_len);
                                key_out = arena_append(&out, tmp, total);
                                free(tmp);
                                if (key_out.ptr == NULL && key_out.len == 0) {
                                    out.status = SGML_STATUS_OOM;
                                    free(fallback);
                                    free_lower_key(lower, stack_buf);
                                    break;
                                }
                            } else {
                                out.status = SGML_STATUS_OOM;
                                free(fallback);
                                free_lower_key(lower, stack_buf);
                                break;
                            }
                        } else {
                            key_out = arena_append(&out, fallback, fallback_len);
                            if (key_out.ptr == NULL && key_out.len == 0) {
                                out.status = SGML_STATUS_OOM;
                                free(fallback);
                                free_lower_key(lower, stack_buf);
                                break;
                            }
                        }
                        free(fallback);
                    }
                }
                free_lower_key(lower, stack_buf);
            }
        } else if (key_in.len > 0 && key_in.ptr) {
            key_out = arena_append(&out, key_in.ptr, key_in.len);
            if (key_out.ptr == NULL && key_out.len == 0) {
                out.status = SGML_STATUS_OOM;
                break;
            }
        }
        new_ev.key = key_out;

        // Normalize value (only for keyval events)
        byte_span val_out = {0};
        if (ev.type == SUB_EVENT_KEYVAL && ev.value.ptr && ev.value.len > 0) {
            byte_span val = ev.value;
            byte_span extracted = {0};
            int used_extract = 0;

            if (klen > 0 && kptr) {
                uint8_t stack_buf[256];
                uint8_t *lower = build_lower_key(kptr, klen, stack_buf, sizeof(stack_buf));
                if (lower) {
                    const map_entry *me = lookup_map(lower, klen);
                    if (me && me->rx != REGEX_NONE) {
                        if (me->rx == REGEX_SEC_ACT) {
                            used_extract = extract_sec_act(val, &extracted);
                        } else if (me->rx == REGEX_SIC) {
                            used_extract = extract_sic(val, &extracted);
                        }
                    }
                    free_lower_key(lower, stack_buf);
                }
            }

            if (used_extract) {
                val_out = arena_append(&out, extracted.ptr, extracted.len);
            } else {
                val_out = arena_append(&out, val.ptr, val.len);
            }
            if (val_out.ptr == NULL && val_out.len == 0) {
                out.status = SGML_STATUS_OOM;
                break;
            }
        }
        new_ev.value = val_out;

        out.events[out.count++] = new_ev;
    }

    return out;
}

void free_standardized_submission_metadata(standardized_submission_metadata *m) {
    if (!m) return;
    free(m->events);
    free(m->arena);
    m->events = NULL;
    m->arena = NULL;
    m->count = 0;
    m->cap = 0;
    m->arena_len = 0;
    m->arena_cap = 0;
}
