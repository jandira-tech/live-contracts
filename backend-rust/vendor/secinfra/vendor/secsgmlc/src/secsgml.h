#ifndef SECSGML_H
#define SECSGML_H

#include <stddef.h>
#include <stdint.h>

// ---------------------------------------------------------------------------
// Core span type -- a view into an existing buffer, no ownership
// ---------------------------------------------------------------------------
typedef struct {
    const uint8_t *ptr;
    size_t len;
} byte_span;

// ---------------------------------------------------------------------------
// Document metadata -- all spans point into the original buffer
// ---------------------------------------------------------------------------
typedef struct {
    byte_span type;
    byte_span sequence;
    byte_span filename;
    byte_span description;
} document_meta;

// ---------------------------------------------------------------------------
// Document -- pointers into original buffer + optional decoded output
// ---------------------------------------------------------------------------
typedef struct {
    document_meta meta;

    // Raw encoded content span (points into original buffer)
    // For uuencoded: points to first encoded line (after "begin 644 ...")
    // For plain: points to the text content
    const uint8_t *content_start;
    size_t         content_len;

    // If uuencoded: malloc'd decoded output, caller must free
    // If not uuencoded: NULL, use content_start/content_len directly
    uint8_t *decoded;
    size_t   decoded_len;

    int is_uuencoded;
} document;

// ---------------------------------------------------------------------------
// Parse result -- owns the docs array, each doc may own decoded buffer
// ---------------------------------------------------------------------------
typedef enum {
    SGML_STATUS_OK = 0,
    SGML_STATUS_OOM = 1,
    SGML_STATUS_TRUNCATED = 2
} sgml_status;

typedef struct {
    document *docs;
    size_t    doc_count;
    size_t    doc_cap;
    sgml_status status;
} sgml_parse_result;

typedef struct {
    // Counts
    size_t doc_count;
    size_t uuencoded_count;
} sgml_parse_stats;

// ---------------------------------------------------------------------------
// Submission metadata (before first <DOCUMENT>)
// ---------------------------------------------------------------------------
typedef enum {
    SUB_EVENT_SECTION_START = 1,
    SUB_EVENT_SECTION_END   = 2,
    SUB_EVENT_KEYVAL        = 3
} submission_event_type;

typedef struct {
    submission_event_type type;
    byte_span key;
    byte_span value;
    int depth;
} submission_event;

typedef struct {
    submission_event *events;
    size_t count;
    size_t cap;
    sgml_status status;
} submission_metadata;

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------
sgml_parse_result    parse_sgml(const uint8_t *buf, size_t len, sgml_parse_stats *stats);
void                 free_sgml_parse_result(sgml_parse_result *r);
submission_metadata  parse_submission_metadata(const uint8_t *buf, size_t len);
void                 free_submission_metadata(submission_metadata *m);

#endif
