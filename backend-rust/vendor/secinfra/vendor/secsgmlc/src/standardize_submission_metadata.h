#ifndef STANDARDIZE_SUBMISSION_METADATA_H
#define STANDARDIZE_SUBMISSION_METADATA_H

#include <stddef.h>
#include <stdint.h>

#include "secsgml.h"

// Standardized submission metadata. Owns its events array and string arena.
typedef struct {
    submission_event *events;
    size_t count;
    size_t cap;
    uint8_t *arena;
    size_t arena_len;
    size_t arena_cap;
    sgml_status status;
} standardized_submission_metadata;

// Returns a standardized copy of submission metadata. Does not mutate input.
standardized_submission_metadata standardize_submission_metadata(const submission_metadata *m);
void free_standardized_submission_metadata(standardized_submission_metadata *m);

#endif
