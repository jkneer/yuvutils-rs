/*
 * Copyright (c) Radzivon Bartoshyk, 10/2024. All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without modification,
 * are permitted provided that the following conditions are met:
 *
 * 1.  Redistributions of source code must retain the above copyright notice, this
 * list of conditions and the following disclaimer.
 *
 * 2.  Redistributions in binary form must reproduce the above copyright notice,
 * this list of conditions and the following disclaimer in the documentation
 * and/or other materials provided with the distribution.
 *
 * 3.  Neither the name of the copyright holder nor the names of its
 * contributors may be used to endorse or promote products derived from
 * this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
 * FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
 * DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
 * SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
 * CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
 * OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 * OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */
use crate::internals::{ProcessedOffset, RowDBiPlanarInversionHandler};
use crate::numerics::{qrshr, to_ne};
use crate::yuv_error::check_rgba_destination;
use crate::yuv_support::*;
use crate::{YuvBiPlanarImage, YuvError};
#[cfg(feature = "rayon")]
use rayon::iter::{IndexedParallelIterator, ParallelIterator};
#[cfg(feature = "rayon")]
use rayon::prelude::{ParallelSlice, ParallelSliceMut};

type RowHandlerFn = unsafe fn(
    y_plane: &[u16],
    uv_plane: &[u16],
    bgra: &mut [u8],
    width: u32,
    range: &YuvChromaRange,
    transform: &CbCrInverseTransform<i32>,
    start_cx: usize,
    start_ux: usize,
) -> ProcessedOffset;

struct RowHandlerBalanced<
    const AR30_LAYOUT: usize,
    const AR30_STORE: usize,
    const NV_ORDER: u8,
    const SAMPLING: u8,
    const ENDIANNESS: u8,
    const BYTES_POSITION: u8,
    const PRECISION: i32,
    const BIT_DEPTH: usize,
> {
    handler: Option<RowHandlerFn>,
}

impl<
        const AR30_LAYOUT: usize,
        const AR30_STORE: usize,
        const NV_ORDER: u8,
        const SAMPLING: u8,
        const ENDIANNESS: u8,
        const BYTES_POSITION: u8,
        const PRECISION: i32,
        const BIT_DEPTH: usize,
    > Default
    for RowHandlerBalanced<
        AR30_LAYOUT,
        AR30_STORE,
        NV_ORDER,
        SAMPLING,
        ENDIANNESS,
        BYTES_POSITION,
        PRECISION,
        BIT_DEPTH,
    >
{
    fn default() -> Self {
        if PRECISION == 14 {
            assert_eq!(PRECISION, 14);
            #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
            {
                use crate::neon::neon_yuv_nv12_p10_to_ar30_row;
                return Self {
                    handler: Some(
                        neon_yuv_nv12_p10_to_ar30_row::<
                            NV_ORDER,
                            SAMPLING,
                            ENDIANNESS,
                            BYTES_POSITION,
                            AR30_LAYOUT,
                            AR30_STORE,
                        >,
                    ),
                };
            }
        }
        Self { handler: None }
    }
}

macro_rules! impl_row_handler_nv10_ar30 {
    ($struct_name:ident) => {
        impl<
                const AR30_LAYOUT: usize,
                const AR30_STORE: usize,
                const NV_ORDER: u8,
                const SAMPLING: u8,
                const ENDIANNESS: u8,
                const BYTES_POSITION: u8,
                const PRECISION: i32,
                const BIT_DEPTH: usize,
            > RowDBiPlanarInversionHandler<u16, u8, i32>
            for $struct_name<
                AR30_LAYOUT,
                AR30_STORE,
                NV_ORDER,
                SAMPLING,
                ENDIANNESS,
                BYTES_POSITION,
                PRECISION,
                BIT_DEPTH,
            >
        {
            fn handle_row(
                &self,
                y_plane: &[u16],
                uv_plane: &[u16],
                rgba: &mut [u8],
                width: u32,
                chroma: YuvChromaRange,
                transform: &CbCrInverseTransform<i32>,
            ) -> ProcessedOffset {
                if let Some(handler) = self.handler {
                    unsafe {
                        return handler(y_plane, uv_plane, rgba, width, &chroma, transform, 0, 0);
                    }
                }
                ProcessedOffset { cx: 0, ux: 0 }
            }
        }
    };
}

impl_row_handler_nv10_ar30!(RowHandlerBalanced);

fn yuv_nv_p10_to_image_impl_d<
    const AR30_LAYOUT: usize,
    const AR30_STORE: usize,
    const NV_ORDER: u8,
    const SAMPLING: u8,
    const ENDIANNESS: u8,
    const BYTES_POSITION: u8,
    const PRECISION: i32,
>(
    image: &YuvBiPlanarImage<u16>,
    ar30: &mut [u8],
    ar30_stride: u32,
    range: YuvRange,
    matrix: YuvStandardMatrix,
    row_handler: impl RowDBiPlanarInversionHandler<u16, u8, i32> + Send + Sync,
) -> Result<(), YuvError> {
    let ar30_layout: Rgb30 = AR30_LAYOUT.into();
    const CN: usize = 4;
    let uv_order: YuvNVOrder = NV_ORDER.into();
    let chroma_subsampling: YuvChromaSubsampling = SAMPLING.into();

    const BIT_DEPTH: usize = 10;

    image.check_constraints(chroma_subsampling)?;
    check_rgba_destination(ar30, ar30_stride, image.width, image.height, CN)?;

    let chroma_range = get_yuv_range(BIT_DEPTH as u32, range);
    let kr_kb = matrix.get_kr_kb();
    let i_transform = search_inverse_transform(
        PRECISION,
        BIT_DEPTH as u32,
        range,
        matrix,
        chroma_range,
        kr_kb,
    );
    let cr_coef = i_transform.cr_coef;
    let cb_coef = i_transform.cb_coef;
    let y_coef = i_transform.y_coef;
    let g_coef_1 = i_transform.g_coeff_1;
    let g_coef_2 = i_transform.g_coeff_2;

    let bias_y = chroma_range.bias_y as i32;
    let bias_uv = chroma_range.bias_uv as i32;

    let msb_shift = 16 - BIT_DEPTH as i32;
    let width = image.width;

    let process_halved_chroma_row = |y_src: &[u16], uv_src: &[u16], rgba: &mut [u8]| {
        let processed =
            row_handler.handle_row(y_src, uv_src, rgba, image.width, chroma_range, &i_transform);
        if processed.cx != image.width as usize {
            for ((rgba, y_src), uv_src) in rgba
                .chunks_exact_mut(CN * 2)
                .zip(y_src.chunks_exact(2))
                .zip(uv_src.chunks_exact(2))
                .skip(processed.cx / 2)
            {
                let y_vl0 = to_ne::<ENDIANNESS, BYTES_POSITION>(y_src[0], msb_shift) as i32;
                let mut cb_value = to_ne::<ENDIANNESS, BYTES_POSITION>(
                    uv_src[uv_order.get_u_position()],
                    msb_shift,
                ) as i32;
                let mut cr_value = to_ne::<ENDIANNESS, BYTES_POSITION>(
                    uv_src[uv_order.get_v_position()],
                    msb_shift,
                ) as i32;

                let y_value0: i32 = (y_vl0 - bias_y) * y_coef;

                cb_value -= bias_uv;
                cr_value -= bias_uv;

                let r_p0 = qrshr::<PRECISION, 10>(y_value0 + cr_coef * cr_value);
                let b_p0 = qrshr::<PRECISION, 10>(y_value0 + cb_coef * cb_value);
                let g_p0 =
                    qrshr::<PRECISION, 10>(y_value0 - g_coef_1 * cr_value - g_coef_2 * cb_value);

                let pixel0 = ar30_layout
                    .pack::<AR30_STORE>(r_p0, g_p0, b_p0)
                    .to_ne_bytes();
                rgba[0] = pixel0[0];
                rgba[1] = pixel0[1];
                rgba[2] = pixel0[2];
                rgba[3] = pixel0[3];

                let y_vl1 = to_ne::<ENDIANNESS, BYTES_POSITION>(y_src[1], msb_shift) as i32;

                let y_value1: i32 = (y_vl1 - bias_y) * y_coef;

                let r_p1 = qrshr::<PRECISION, 10>(y_value1 + cr_coef * cr_value);
                let b_p1 = qrshr::<PRECISION, 10>(y_value1 + cb_coef * cb_value);
                let g_p1 =
                    qrshr::<PRECISION, 10>(y_value1 - g_coef_1 * cr_value - g_coef_2 * cb_value);

                let pixel1 = ar30_layout
                    .pack::<AR30_STORE>(r_p1, g_p1, b_p1)
                    .to_ne_bytes();
                rgba[4] = pixel1[0];
                rgba[5] = pixel1[1];
                rgba[6] = pixel1[2];
                rgba[7] = pixel1[3];
            }

            if width & 1 != 0 {
                let rgba = rgba.chunks_exact_mut(CN * 2).into_remainder();
                let rgba = &mut rgba[0..CN];
                let uv_src = uv_src.chunks_exact(2).last().unwrap();
                let y_src = y_src.chunks_exact(2).remainder();

                let y_vl0 = to_ne::<ENDIANNESS, BYTES_POSITION>(y_src[0], msb_shift) as i32;
                let y_value0: i32 = (y_vl0 - bias_y) * y_coef;
                let mut cb_value = to_ne::<ENDIANNESS, BYTES_POSITION>(
                    uv_src[uv_order.get_u_position()],
                    msb_shift,
                ) as i32;
                let mut cr_value = to_ne::<ENDIANNESS, BYTES_POSITION>(
                    uv_src[uv_order.get_v_position()],
                    msb_shift,
                ) as i32;

                cb_value -= bias_uv;
                cr_value -= bias_uv;

                let r_p0 = qrshr::<PRECISION, 10>(y_value0 + cr_coef * cr_value);
                let b_p0 = qrshr::<PRECISION, 10>(y_value0 + cb_coef * cb_value);
                let g_p0 =
                    qrshr::<PRECISION, 10>(y_value0 - g_coef_1 * cr_value - g_coef_2 * cb_value);

                let pixel0 = ar30_layout
                    .pack::<AR30_STORE>(r_p0, g_p0, b_p0)
                    .to_ne_bytes();
                rgba[0] = pixel0[0];
                rgba[1] = pixel0[1];
                rgba[2] = pixel0[2];
                rgba[3] = pixel0[3];
            }
        }
    };

    let y_stride = image.y_stride;
    let uv_stride = image.uv_stride;
    let y_plane = image.y_plane;
    let uv_plane = image.uv_plane;

    if chroma_subsampling == YuvChromaSubsampling::Yuv444 {
        let iter;
        #[cfg(feature = "rayon")]
        {
            iter = y_plane
                .par_chunks_exact(y_stride as usize)
                .zip(uv_plane.par_chunks_exact(uv_stride as usize))
                .zip(ar30.par_chunks_exact_mut(ar30_stride as usize));
        }
        #[cfg(not(feature = "rayon"))]
        {
            iter = y_plane
                .chunks_exact(y_stride as usize)
                .zip(uv_plane.chunks_exact(uv_stride as usize))
                .zip(ar30.chunks_exact_mut(ar30_stride as usize));
        }
        iter.for_each(|((y_src, uv_src), rgba)| {
            let y_src = &y_src[0..image.width as usize];
            let processed = row_handler.handle_row(
                y_src,
                uv_src,
                rgba,
                image.width,
                chroma_range,
                &i_transform,
            );
            if processed.cx != image.width as usize {
                for ((rgba, &y_src), uv_src) in rgba
                    .chunks_exact_mut(CN)
                    .zip(y_src.iter())
                    .zip(uv_src.chunks_exact(2))
                    .skip(processed.cx)
                {
                    let y_vl = to_ne::<ENDIANNESS, BYTES_POSITION>(y_src, msb_shift) as i32;
                    let mut cb_value = to_ne::<ENDIANNESS, BYTES_POSITION>(
                        uv_src[uv_order.get_u_position()],
                        msb_shift,
                    ) as i32;
                    let mut cr_value = to_ne::<ENDIANNESS, BYTES_POSITION>(
                        uv_src[uv_order.get_v_position()],
                        msb_shift,
                    ) as i32;

                    let y_value: i32 = (y_vl - bias_y) * y_coef;

                    cb_value -= bias_uv;
                    cr_value -= bias_uv;

                    let r_p = qrshr::<PRECISION, 10>(y_value + cr_coef * cr_value);
                    let b_p = qrshr::<PRECISION, 10>(y_value + cb_coef * cb_value);
                    let g_p =
                        qrshr::<PRECISION, 10>(y_value - g_coef_1 * cr_value - g_coef_2 * cb_value);

                    let pixel0 = ar30_layout.pack::<AR30_STORE>(r_p, g_p, b_p).to_ne_bytes();
                    rgba[0] = pixel0[0];
                    rgba[1] = pixel0[1];
                    rgba[2] = pixel0[2];
                    rgba[3] = pixel0[3];
                }
            }
        });
    } else if chroma_subsampling == YuvChromaSubsampling::Yuv422 {
        let iter;
        #[cfg(feature = "rayon")]
        {
            iter = y_plane
                .par_chunks_exact(y_stride as usize)
                .zip(uv_plane.par_chunks_exact(uv_stride as usize))
                .zip(ar30.par_chunks_exact_mut(ar30_stride as usize));
        }
        #[cfg(not(feature = "rayon"))]
        {
            iter = y_plane
                .chunks_exact(y_stride as usize)
                .zip(uv_plane.chunks_exact(uv_stride as usize))
                .zip(ar30.chunks_exact_mut(ar30_stride as usize));
        }
        iter.for_each(|((y_src, uv_src), rgba)| {
            process_halved_chroma_row(
                &y_src[0..image.width as usize],
                &uv_src[0..(image.width as usize).div_ceil(2) * 2],
                &mut rgba[0..image.width as usize * CN],
            );
        });
    } else if chroma_subsampling == YuvChromaSubsampling::Yuv420 {
        let iter;
        #[cfg(feature = "rayon")]
        {
            iter = y_plane
                .par_chunks_exact(y_stride as usize * 2)
                .zip(uv_plane.par_chunks_exact(uv_stride as usize))
                .zip(ar30.par_chunks_exact_mut(ar30_stride as usize * 2));
        }
        #[cfg(not(feature = "rayon"))]
        {
            iter = y_plane
                .chunks_exact(y_stride as usize * 2)
                .zip(uv_plane.chunks_exact(uv_stride as usize))
                .zip(ar30.chunks_exact_mut(ar30_stride as usize * 2));
        }
        iter.for_each(|((y_src, uv_src), rgba)| {
            for (y_src, rgba) in y_src
                .chunks_exact(y_stride as usize)
                .zip(rgba.chunks_exact_mut(ar30_stride as usize))
            {
                process_halved_chroma_row(
                    &y_src[0..image.width as usize],
                    &uv_src[0..(image.width as usize).div_ceil(2) * 2],
                    &mut rgba[0..image.width as usize * CN],
                );
            }
        });
        if image.height & 1 != 0 {
            let y_src = y_plane.chunks_exact(y_stride as usize * 2).remainder();
            let uv_src = uv_plane.chunks_exact(uv_stride as usize).last().unwrap();
            let rgba = ar30
                .chunks_exact_mut(ar30_stride as usize * 2)
                .into_remainder();
            process_halved_chroma_row(
                &y_src[0..image.width as usize],
                &uv_src[0..(image.width as usize).div_ceil(2) * 2],
                &mut rgba[0..image.width as usize * CN],
            );
        }
    } else {
        unreachable!();
    }

    Ok(())
}

#[inline]
fn yuv_nv_p10_to_image_impl<
    const AR30_LAYOUT: usize,
    const NV_ORDER: u8,
    const SAMPLING: u8,
    const ENDIANNESS: u8,
    const BYTES_POSITION: u8,
>(
    image: &YuvBiPlanarImage<u16>,
    bgra: &mut [u8],
    bgra_stride: u32,
    order: Rgb30ByteOrder,
    range: YuvRange,
    matrix: YuvStandardMatrix,
) -> Result<(), YuvError> {
    match order {
        Rgb30ByteOrder::Host => yuv_nv_p10_to_image_impl_d::<
            AR30_LAYOUT,
            { Rgb30ByteOrder::Host as usize },
            NV_ORDER,
            SAMPLING,
            ENDIANNESS,
            BYTES_POSITION,
            14,
        >(
            image,
            bgra,
            bgra_stride,
            range,
            matrix,
            RowHandlerBalanced::<
                AR30_LAYOUT,
                { Rgb30ByteOrder::Host as usize },
                NV_ORDER,
                SAMPLING,
                ENDIANNESS,
                BYTES_POSITION,
                14,
                10,
            >::default(),
        ),
        Rgb30ByteOrder::Network => yuv_nv_p10_to_image_impl_d::<
            AR30_LAYOUT,
            { Rgb30ByteOrder::Network as usize },
            NV_ORDER,
            SAMPLING,
            ENDIANNESS,
            BYTES_POSITION,
            14,
        >(
            image,
            bgra,
            bgra_stride,
            range,
            matrix,
            RowHandlerBalanced::<
                AR30_LAYOUT,
                { Rgb30ByteOrder::Network as usize },
                NV_ORDER,
                SAMPLING,
                ENDIANNESS,
                BYTES_POSITION,
                14,
                10,
            >::default(),
        ),
    }
}

macro_rules! yuv_to_ab30_conversion {
    (
        $method_name:ident,
        $ab_format:expr,
        $chroma_subsampling:expr,
        $chroma_interleaved_order: expr,
        $uv_format_name: expr,
        $ab_format_name: expr
    ) => {
        #[doc = concat!("Convert YUV ", $uv_format_name, " format with 10-bit pixel format to ", $ab_format_name, " format.

This function takes YUV ", $uv_format_name, " data with 10-bit precision
and converts it to ", $ab_format_name," format.

# Arguments

* `bi_planar_image` - Source Bi-Planar 10-bit image.
* `dst` - A mutable slice to store the converted ", $ab_format_name, " data.
* `dst_stride` - The stride for the ", $ab_format_name, " image data.
* `byte_order` - see [Rgb30ByteOrder] for more info
* `endianness` - The endianness of stored bytes
* `bytes_packing` - see [YuvBytesPacking] for more info.

# Panics

This function panics if the lengths of the planes or the input ", $ab_format_name," data are not valid based
on the specified width, height, and strides, or if invalid YUV range or matrix is provided.")]
        pub fn $method_name(
            bi_planar_image: &YuvBiPlanarImage<u16>,
            dst: &mut [u8],
            dst_stride: u32,
            byte_order: Rgb30ByteOrder,
            range: YuvRange,
            matrix: YuvStandardMatrix,
            endianness: YuvEndianness,
            bytes_packing: YuvBytesPacking,
        ) -> Result<(), YuvError> {
            let dispatcher = match endianness {
                YuvEndianness::BigEndian => match bytes_packing {
                    YuvBytesPacking::MostSignificantBytes => {
                        yuv_nv_p10_to_image_impl::<
                            { $ab_format as usize },
                            { $chroma_interleaved_order as u8 },
                            { $chroma_subsampling as u8 },
                            { YuvEndianness::BigEndian as u8 },
                            { YuvBytesPacking::MostSignificantBytes as u8 },
                        >
                    }
                    YuvBytesPacking::LeastSignificantBytes => {
                        yuv_nv_p10_to_image_impl::<
                            { $ab_format as usize },
                            { YuvNVOrder::UV as u8 },
                            { $chroma_subsampling as u8 },
                            { YuvEndianness::BigEndian as u8 },
                            { YuvBytesPacking::LeastSignificantBytes as u8 },
                        >
                    }
                },
                YuvEndianness::LittleEndian => match bytes_packing {
                    YuvBytesPacking::MostSignificantBytes => {
                        yuv_nv_p10_to_image_impl::<
                            { $ab_format as usize },
                            { $chroma_interleaved_order as u8 },
                            { $chroma_subsampling as u8 },
                            { YuvEndianness::LittleEndian as u8 },
                            { YuvBytesPacking::MostSignificantBytes as u8 },
                        >
                    }
                    YuvBytesPacking::LeastSignificantBytes => {
                        yuv_nv_p10_to_image_impl::<
                            { $ab_format as usize },
                            { $chroma_interleaved_order as u8 },
                            { $chroma_subsampling as u8 },
                            { YuvEndianness::LittleEndian as u8 },
                            { YuvBytesPacking::LeastSignificantBytes as u8 },
                        >
                    }
                },
            };
            dispatcher(
                bi_planar_image,
                dst,
                dst_stride,
                byte_order,
                range,
                matrix,
            )
        }
    };
}

yuv_to_ab30_conversion!(
    yuv_nv12_p10_to_ab30,
    Rgb30::Ab30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::UV,
    "NV12",
    "AB30"
);
yuv_to_ab30_conversion!(
    yuv_nv12_p10_to_ar30,
    Rgb30::Ar30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::UV,
    "NV12",
    "AR30"
);
yuv_to_ab30_conversion!(
    yuv_nv12_p10_to_ra30,
    Rgb30::Ra30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::UV,
    "NV12",
    "RA30"
);
yuv_to_ab30_conversion!(
    yuv_nv12_p10_to_ba30,
    Rgb30::Ba30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::UV,
    "NV12",
    "BA30"
);

yuv_to_ab30_conversion!(
    yuv_nv21_p10_to_ab30,
    Rgb30::Ab30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::VU,
    "NV21",
    "AB30"
);
yuv_to_ab30_conversion!(
    yuv_nv21_p10_to_ar30,
    Rgb30::Ar30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::VU,
    "NV21",
    "AR30"
);
yuv_to_ab30_conversion!(
    yuv_nv21_p10_to_ra30,
    Rgb30::Ra30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::VU,
    "NV21",
    "RA30"
);
yuv_to_ab30_conversion!(
    yuv_nv21_p10_to_ba30,
    Rgb30::Ba30,
    YuvChromaSubsampling::Yuv420,
    YuvNVOrder::VU,
    "NV21",
    "BA30"
);

yuv_to_ab30_conversion!(
    yuv_nv16_p10_to_ab30,
    Rgb30::Ab30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::UV,
    "NV16",
    "AB30"
);
yuv_to_ab30_conversion!(
    yuv_nv16_p10_to_ar30,
    Rgb30::Ar30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::UV,
    "NV16",
    "AR30"
);
yuv_to_ab30_conversion!(
    yuv_nv16_p10_to_ra30,
    Rgb30::Ra30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::UV,
    "NV16",
    "RA30"
);
yuv_to_ab30_conversion!(
    yuv_nv16_p10_to_ba30,
    Rgb30::Ba30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::UV,
    "NV16",
    "BA30"
);

yuv_to_ab30_conversion!(
    yuv_nv61_p10_to_ab30,
    Rgb30::Ab30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::VU,
    "NV61",
    "AB30"
);
yuv_to_ab30_conversion!(
    yuv_nv61_p10_to_ar30,
    Rgb30::Ar30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::VU,
    "NV61",
    "AR30"
);
yuv_to_ab30_conversion!(
    yuv_nv61_p10_to_ra30,
    Rgb30::Ra30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::VU,
    "NV61",
    "RA30"
);
yuv_to_ab30_conversion!(
    yuv_nv61_p10_to_ba30,
    Rgb30::Ba30,
    YuvChromaSubsampling::Yuv422,
    YuvNVOrder::VU,
    "NV61",
    "BA30"
);
