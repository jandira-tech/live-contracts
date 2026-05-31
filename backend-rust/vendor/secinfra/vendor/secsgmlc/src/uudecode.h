#ifndef UUDECODE_H
#define UUDECODE_H

#include <stddef.h>
#include <stdint.h>

// Decode a uuencoded buffer (no begin/end lines) into out.
// Returns number of bytes written (may be less than out_cap if input is larger).
size_t uudecode(const uint8_t *in, size_t in_len, uint8_t *out, size_t out_cap);

#endif
