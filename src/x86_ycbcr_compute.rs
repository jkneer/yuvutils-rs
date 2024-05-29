/*
 * // Copyright (c) the Radzivon Bartoshyk. All rights reserved.
 * //
 * // Use of this source code is governed by a BSD-style
 * // license that can be found in the LICENSE file.
 */

#[allow(unused_imports)]
use crate::x86_simd_support::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
#[inline(always)]
#[allow(dead_code)]
pub unsafe fn avx2_rgb_to_ycbcr(
    r: __m256i,
    g: __m256i,
    b: __m256i,
    bias: __m256i,
    coeff_r: __m256i,
    coeff_g: __m256i,
    coeff_b: __m256i,
) -> __m256i {
    let r_l = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(r));
    let g_l = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(g));
    let b_l = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(b));

    let vl = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        bias,
        _mm256_add_epi32(
            _mm256_add_epi32(
                _mm256_mullo_epi32(coeff_r, r_l),
                _mm256_mullo_epi32(coeff_g, g_l),
            ),
            _mm256_mullo_epi32(coeff_b, b_l),
        ),
    ));

    let r_h = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(r));
    let g_h = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(g));
    let b_h = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(b));

    let vh = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        bias,
        _mm256_add_epi32(
            _mm256_add_epi32(
                _mm256_mullo_epi32(coeff_r, r_h),
                _mm256_mullo_epi32(coeff_g, g_h),
            ),
            _mm256_mullo_epi32(coeff_b, b_h),
        ),
    ));

    let k = _mm256_packus_epi32(vl, vh);
    const MASK: i32 = shuffle(3, 1, 2, 0);
    _mm256_permute4x64_epi64::<MASK>(k)
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
#[allow(dead_code)]
pub unsafe fn sse_rgb_to_ycbcr(
    r: __m128i,
    g: __m128i,
    b: __m128i,
    bias: __m128i,
    coeff_r: __m128i,
    coeff_g: __m128i,
    coeff_b: __m128i,
) -> __m128i {
    let r_l = _mm_cvtepi16_epi32(r);
    let g_l = _mm_cvtepi16_epi32(g);
    let b_l = _mm_cvtepi16_epi32(b);

    let vl = _mm_srai_epi32::<8>(_mm_add_epi32(
        bias,
        _mm_add_epi32(
            _mm_add_epi32(_mm_mullo_epi32(coeff_r, r_l), _mm_mullo_epi32(coeff_g, g_l)),
            _mm_mullo_epi32(coeff_b, b_l),
        ),
    ));

    let r_h = sse_promote_i16_toi32(r);
    let g_h = sse_promote_i16_toi32(g);
    let b_h = sse_promote_i16_toi32(b);

    let vh = _mm_srai_epi32::<8>(_mm_add_epi32(
        bias,
        _mm_add_epi32(
            _mm_add_epi32(_mm_mullo_epi32(coeff_r, r_h), _mm_mullo_epi32(coeff_g, g_h)),
            _mm_mullo_epi32(coeff_b, b_h),
        ),
    ));

    _mm_packus_epi32(vl, vh)
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
#[allow(dead_code)]
pub unsafe fn sse_rgb_to_ycgco(
    r: __m128i,
    g: __m128i,
    b: __m128i,
    y_reduction: __m128i,
    uv_reduction: __m128i,
    y_bias: __m128i,
    uv_bias: __m128i,
) -> (__m128i, __m128i, __m128i) {
    let mut r_l = _mm_cvtepi16_epi32(r);
    let mut g_l = _mm_cvtepi16_epi32(g);
    let mut b_l = _mm_cvtepi16_epi32(b);

    let hg_0 = _mm_srai_epi32::<1>(_mm_mullo_epi32(g_l, y_reduction));

    let yl_0 = _mm_srai_epi32::<8>(_mm_add_epi32(
        _mm_add_epi32(
            _mm_srai_epi32::<2>(_mm_add_epi32(
                _mm_mullo_epi32(r_l, y_reduction),
                _mm_mullo_epi32(b_l, y_reduction),
            )),
            hg_0,
        ),
        y_bias,
    ));

    r_l = _mm_mullo_epi32(r_l, uv_reduction);
    g_l = _mm_mullo_epi32(g_l, uv_reduction);
    b_l = _mm_mullo_epi32(b_l, uv_reduction);

    let cg_l = _mm_srai_epi32::<8>(_mm_add_epi32(
        _mm_sub_epi32(
            _mm_srai_epi32::<1>(g_l),
            _mm_srai_epi32::<2>(_mm_add_epi32(r_l, b_l)),
        ),
        uv_bias,
    ));

    let co_l = _mm_srai_epi32::<8>(_mm_add_epi32(
        _mm_srai_epi32::<1>(_mm_sub_epi32(r_l, b_l)),
        uv_bias,
    ));

    let mut r_h = sse_promote_i16_toi32(r);
    let mut g_h = sse_promote_i16_toi32(g);
    let mut b_h = sse_promote_i16_toi32(b);

    let hg_1 = _mm_srai_epi32::<1>(_mm_mullo_epi32(g_h, y_reduction));

    let yh_0 = _mm_srai_epi32::<8>(_mm_add_epi32(
        _mm_add_epi32(
            _mm_srai_epi32::<2>(_mm_add_epi32(
                _mm_mullo_epi32(r_h, y_reduction),
                _mm_mullo_epi32(b_h, y_reduction),
            )),
            hg_1,
        ),
        y_bias,
    ));

    r_h = _mm_mullo_epi32(r_h, uv_reduction);
    g_h = _mm_mullo_epi32(g_h, uv_reduction);
    b_h = _mm_mullo_epi32(b_h, uv_reduction);

    let cg_h = _mm_srai_epi32::<8>(_mm_add_epi32(
        _mm_sub_epi32(
            _mm_srai_epi32::<1>(g_h),
            _mm_srai_epi32::<2>(_mm_add_epi32(r_h, b_h)),
        ),
        uv_bias,
    ));

    let co_h = _mm_srai_epi32::<8>(_mm_add_epi32(
        _mm_srai_epi32::<1>(_mm_sub_epi32(r_h, b_h)),
        uv_bias,
    ));

    (
        _mm_packus_epi32(yl_0, yh_0),
        _mm_packus_epi32(cg_l, cg_h),
        _mm_packus_epi32(co_l, co_h),
    )
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
#[allow(dead_code)]
pub unsafe fn avx2_rgb_to_ycgco(
    r: __m256i,
    g: __m256i,
    b: __m256i,
    y_reduction: __m256i,
    uv_reduction: __m256i,
    y_bias: __m256i,
    uv_bias: __m256i,
) -> (__m256i, __m256i, __m256i) {
    let mut r_l = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(r));
    let mut g_l = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(g));
    let mut b_l = _mm256_cvtepi16_epi32(_mm256_castsi256_si128(b));

    let hg_0 = _mm256_srai_epi32::<1>(_mm256_mullo_epi32(g_l, y_reduction));

    let yl_0 = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        _mm256_add_epi32(
            _mm256_srai_epi32::<2>(_mm256_add_epi32(
                _mm256_mullo_epi32(r_l, y_reduction),
                _mm256_mullo_epi32(b_l, y_reduction),
            )),
            hg_0,
        ),
        y_bias,
    ));

    r_l = _mm256_mullo_epi32(r_l, uv_reduction);
    g_l = _mm256_mullo_epi32(g_l, uv_reduction);
    b_l = _mm256_mullo_epi32(b_l, uv_reduction);

    let cg_l = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        _mm256_sub_epi32(
            _mm256_srai_epi32::<1>(g_l),
            _mm256_srai_epi32::<2>(_mm256_add_epi32(r_l, b_l)),
        ),
        uv_bias,
    ));

    let co_l = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        _mm256_srai_epi32::<1>(_mm256_sub_epi32(r_l, b_l)),
        uv_bias,
    ));

    let mut r_h = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(r));
    let mut g_h = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(g));
    let mut b_h = _mm256_cvtepi16_epi32(_mm256_extracti128_si256::<1>(b));

    let hg_1 = _mm256_srai_epi32::<1>(_mm256_mullo_epi32(g_h, y_reduction));

    let yh_0 = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        _mm256_add_epi32(
            _mm256_srai_epi32::<2>(_mm256_add_epi32(
                _mm256_mullo_epi32(r_h, y_reduction),
                _mm256_mullo_epi32(b_h, y_reduction),
            )),
            hg_1,
        ),
        y_bias,
    ));

    r_h = _mm256_mullo_epi32(r_h, uv_reduction);
    g_h = _mm256_mullo_epi32(g_h, uv_reduction);
    b_h = _mm256_mullo_epi32(b_h, uv_reduction);

    let cg_h = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        _mm256_sub_epi32(
            _mm256_srai_epi32::<1>(g_h),
            _mm256_srai_epi32::<2>(_mm256_add_epi32(r_h, b_h)),
        ),
        uv_bias,
    ));

    let co_h = _mm256_srai_epi32::<8>(_mm256_add_epi32(
        _mm256_srai_epi32::<1>(_mm256_sub_epi32(r_h, b_h)),
        uv_bias,
    ));

    const MASK: i32 = shuffle(3, 1, 2, 0);
    (
        _mm256_permute4x64_epi64::<MASK>(_mm256_packus_epi32(yl_0, yh_0)),
        _mm256_permute4x64_epi64::<MASK>(_mm256_packus_epi32(cg_l, cg_h)),
        _mm256_permute4x64_epi64::<MASK>(_mm256_packus_epi32(co_l, co_h)),
    )
}
