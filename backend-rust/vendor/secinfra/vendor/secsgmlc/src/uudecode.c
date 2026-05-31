#include "uudecode.h"
#include <stdint.h>
#include <stddef.h>
#include <string.h>

// ---------------------------------------------------------------------------
// SIMD path selection: AVX2 > SSE2 > NEON > scalar
// ---------------------------------------------------------------------------

#if defined(__AVX2__)
#include <immintrin.h>
#define UUDECODE_AVX2

#elif defined(__SSE2__)
#include <emmintrin.h>
#define UUDECODE_SSE2

#elif defined(__ARM_NEON)
#include <arm_neon.h>
#define UUDECODE_NEON

#endif

// ---------------------------------------------------------------------------
// Newline scan
// ---------------------------------------------------------------------------

#ifdef UUDECODE_AVX2
static inline const uint8_t *find_newline(const uint8_t *p, const uint8_t *end)
{
    __m256i nl = _mm256_set1_epi8('\n');
    __m256i cr = _mm256_set1_epi8('\r');
    while (p + 32 <= end)
    {
        __m256i chunk = _mm256_loadu_si256((const __m256i *)p);
        uint32_t mask = (uint32_t)(_mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, nl)) |
                                   _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, cr)));
        if (mask)
            return p + __builtin_ctz(mask);
        p += 32;
    }
    while (p < end && *p != '\n' && *p != '\r')
        p++;
    return p;
}

#elif defined(UUDECODE_SSE2)
static inline const uint8_t *find_newline(const uint8_t *p, const uint8_t *end)
{
    __m128i nl = _mm_set1_epi8('\n');
    __m128i cr = _mm_set1_epi8('\r');
    while (p + 16 <= end)
    {
        __m128i chunk = _mm_loadu_si128((const __m128i *)p);
        uint32_t mask = (uint32_t)(_mm_movemask_epi8(_mm_cmpeq_epi8(chunk, nl)) |
                                   _mm_movemask_epi8(_mm_cmpeq_epi8(chunk, cr)));
        if (mask)
            return p + __builtin_ctz(mask);
        p += 16;
    }
    while (p < end && *p != '\n' && *p != '\r')
        p++;
    return p;
}

#elif defined(UUDECODE_NEON)
static inline const uint8_t *find_newline(const uint8_t *p, const uint8_t *end)
{
    uint8x16_t nl = vdupq_n_u8('\n');
    uint8x16_t cr = vdupq_n_u8('\r');
    while (p + 16 <= end)
    {
        uint8x16_t chunk = vld1q_u8(p);
        uint8x16_t match = vorrq_u8(vceqq_u8(chunk, nl), vceqq_u8(chunk, cr));
        // Collapse to 64-bit to check for any match
        uint64_t lo, hi;
        lo = vgetq_lane_u64(vreinterpretq_u64_u8(match), 0);
        hi = vgetq_lane_u64(vreinterpretq_u64_u8(match), 1);
        if (lo | hi)
        {
            // Find exact position scalar
            for (int k = 0; k < 16; k++)
            {
                if (p[k] == '\n' || p[k] == '\r')
                    return p + k;
            }
        }
        p += 16;
    }
    while (p < end && *p != '\n' && *p != '\r')
        p++;
    return p;
}

#else
static inline const uint8_t *find_newline(const uint8_t *p, const uint8_t *end)
{
    while (p < end && *p != '\n' && *p != '\r')
        p++;
    return p;
}
#endif

// ---------------------------------------------------------------------------
// Full line decode: 60 encoded bytes -> 45 decoded bytes
// ---------------------------------------------------------------------------

#ifdef UUDECODE_AVX2
static inline void decode_full_line(const uint8_t *in, uint8_t *out, size_t *out_pos)
{
    __m256i sub32 = _mm256_set1_epi8(32);
    __m256i mask6 = _mm256_set1_epi8(0x3f);

    uint8_t tmp[64];

    // Two 32-byte loads cover all 60 encoded bytes (second load overlaps)
    __m256i c0 = _mm256_and_si256(_mm256_sub_epi8(_mm256_loadu_si256((const __m256i *)(in)), sub32), mask6);
    __m256i c1 = _mm256_and_si256(_mm256_sub_epi8(_mm256_loadu_si256((const __m256i *)(in + 32)), sub32), mask6);
    _mm256_storeu_si256((__m256i *)tmp, c0);
    _mm256_storeu_si256((__m256i *)(tmp + 32), c1);

    uint8_t *o = out + *out_pos;
    for (int g = 0; g < 15; g++)
    {
        uint8_t da = tmp[g * 4 + 0], db = tmp[g * 4 + 1], dc = tmp[g * 4 + 2], dd = tmp[g * 4 + 3];
        o[g * 3 + 0] = (da << 2) | (db >> 4);
        o[g * 3 + 1] = (db << 4) | (dc >> 2);
        o[g * 3 + 2] = (dc << 6) | dd;
    }
    *out_pos += 45;
}

#elif defined(UUDECODE_SSE2)
static inline void decode_full_line(const uint8_t *in, uint8_t *out, size_t *out_pos)
{
    __m128i sub32 = _mm_set1_epi8(32);
    __m128i mask6 = _mm_set1_epi8(0x3f);

    uint8_t tmp[64];

    // Four 16-byte loads cover 60 bytes (last load at offset 44, overlaps by 4)
    __m128i c0 = _mm_and_si128(_mm_sub_epi8(_mm_loadu_si128((const __m128i *)(in)), sub32), mask6);
    __m128i c1 = _mm_and_si128(_mm_sub_epi8(_mm_loadu_si128((const __m128i *)(in + 16)), sub32), mask6);
    __m128i c2 = _mm_and_si128(_mm_sub_epi8(_mm_loadu_si128((const __m128i *)(in + 32)), sub32), mask6);
    __m128i c3 = _mm_and_si128(_mm_sub_epi8(_mm_loadu_si128((const __m128i *)(in + 44)), sub32), mask6);
    _mm_storeu_si128((__m128i *)tmp, c0);
    _mm_storeu_si128((__m128i *)(tmp + 16), c1);
    _mm_storeu_si128((__m128i *)(tmp + 32), c2);
    _mm_storeu_si128((__m128i *)(tmp + 44), c3);

    uint8_t *o = out + *out_pos;
    for (int g = 0; g < 15; g++)
    {
        uint8_t da = tmp[g * 4 + 0], db = tmp[g * 4 + 1], dc = tmp[g * 4 + 2], dd = tmp[g * 4 + 3];
        o[g * 3 + 0] = (da << 2) | (db >> 4);
        o[g * 3 + 1] = (db << 4) | (dc >> 2);
        o[g * 3 + 2] = (dc << 6) | dd;
    }
    *out_pos += 45;
}

#elif defined(UUDECODE_NEON)
static inline void decode_full_line(const uint8_t *in, uint8_t *out, size_t *out_pos)
{
    uint8x16_t sub32 = vdupq_n_u8(32);
    uint8x16_t mask6 = vdupq_n_u8(0x3f);

    uint8_t tmp[64];

    // Four 16-byte loads
    vst1q_u8(tmp, vandq_u8(vsubq_u8(vld1q_u8(in), sub32), mask6));
    vst1q_u8(tmp + 16, vandq_u8(vsubq_u8(vld1q_u8(in + 16), sub32), mask6));
    vst1q_u8(tmp + 32, vandq_u8(vsubq_u8(vld1q_u8(in + 32), sub32), mask6));
    vst1q_u8(tmp + 44, vandq_u8(vsubq_u8(vld1q_u8(in + 44), sub32), mask6));

    uint8_t *o = out + *out_pos;
    for (int g = 0; g < 15; g++)
    {
        uint8_t da = tmp[g * 4 + 0], db = tmp[g * 4 + 1], dc = tmp[g * 4 + 2], dd = tmp[g * 4 + 3];
        o[g * 3 + 0] = (da << 2) | (db >> 4);
        o[g * 3 + 1] = (db << 4) | (dc >> 2);
        o[g * 3 + 2] = (dc << 6) | dd;
    }
    *out_pos += 45;
}

#else
static inline void decode_full_line(const uint8_t *in, uint8_t *out, size_t *out_pos)
{
    uint8_t *o = out + *out_pos;
    for (int g = 0; g < 15; g++)
    {
        uint8_t da = (in[g * 4 + 0] - 32) & 0x3f;
        uint8_t db = (in[g * 4 + 1] - 32) & 0x3f;
        uint8_t dc = (in[g * 4 + 2] - 32) & 0x3f;
        uint8_t dd = (in[g * 4 + 3] - 32) & 0x3f;
        o[g * 3 + 0] = (da << 2) | (db >> 4);
        o[g * 3 + 1] = (db << 4) | (dc >> 2);
        o[g * 3 + 2] = (dc << 6) | dd;
    }
    *out_pos += 45;
}
#endif

// ---------------------------------------------------------------------------
// Main decode loop
// ---------------------------------------------------------------------------

size_t uudecode(const uint8_t *in, size_t in_len, uint8_t *out, size_t out_cap)
{
    size_t i = 0;
    size_t out_pos = 0;
    const uint8_t *buf = in;
    const uint8_t *end = in + in_len;

    while (i < in_len)
    {
        uint8_t len_char = buf[i];

        // Skip blank lines / CRLF
        if (len_char == '\n' || len_char == '\r')
        {
            i++;
            continue;
        }

        // Zero-length line = end of uuencoded block
        int nbytes = (len_char - 32) & 0x3f;
        if (nbytes == 0)
            break;

        i++; // consume length char

        // Find end of line (SIMD-accelerated)
        const uint8_t *line_start = buf + i;
        const uint8_t *line_end = find_newline(line_start, end);
        size_t line_len = (size_t)(line_end - line_start);

        // Advance i past line and newline
        i += line_len;
        if (i < in_len && buf[i] == '\r')
            i++;
        if (i < in_len && buf[i] == '\n')
            i++;

        int out_full = 0;
        if (out_pos + (size_t)nbytes > out_cap)
        {
            nbytes = (int)(out_cap - out_pos);
            out_full = 1;
        }
        if (nbytes <= 0)
            return out_pos;

        // --- FAST PATH: full line, >= 60 encoded chars ---
        if (!out_full && len_char == 'M' && line_len >= 60 && (line_start + 64 <= end) &&
            (out_pos + 45 <= out_cap))
        {
            decode_full_line(line_start, out, &out_pos);
            continue;
        }

        // --- SLOW PATH: partial or short line ---
        const uint8_t *p = line_start;
        int remaining = nbytes;
        uint32_t leftchar = 0;
        int leftbits = 0;
        while (remaining > 0)
        {
            uint8_t ch = (p < line_end) ? ((*p++ - 32) & 0x3f) : 0;
            leftchar = (leftchar << 6) | ch;
            leftbits += 6;
            if (leftbits >= 8)
            {
                leftbits -= 8;
                if (out_pos >= out_cap)
                    return out_pos;
                out[out_pos++] = (leftchar >> leftbits) & 0xff;
                leftchar &= (1 << leftbits) - 1;
                remaining--;
            }
        }

        if (out_full)
            return out_pos;
    }

    return out_pos;
}
