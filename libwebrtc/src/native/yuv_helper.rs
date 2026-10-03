// Copyright 2025 LiveKit, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(clippy::too_many_arguments)]

use crate::video_frame::VideoBuffer;
use webrtc_sys::yuv_helper as yuv_sys;

/// RGB-to-YUV conversion matrix and numeric range for 8-bit planar video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum YuvMatrix {
    /// BT.601 with limited range.
    Bt601Limited,
    /// BT.601 with full range.
    Bt601Full,
    /// BT.709 with limited range.
    Bt709Limited,
    /// BT.709 with full range.
    Bt709Full,
}

/// Converts little-endian ARGB (BGRA bytes) into an allocated I420 buffer.
///
/// Uses the selected matrix/range without changing RGB primaries or transfer.
/// Panics if the source dimensions, stride or slice cannot cover the output.
pub fn argb_to_i420_with_matrix(
    src: &[u8],
    stride: u32,
    dst: &mut crate::video_frame::I420Buffer,
    matrix: YuvMatrix,
) {
    let (width, height) = (dst.width(), dst.height());
    assert!(width > 0 && height > 0 && width <= i32::MAX as u32 && height <= i32::MAX as u32);
    assert!(stride <= i32::MAX as u32 && u64::from(stride) >= u64::from(width) * 4);
    let required = u64::from(stride) * u64::from(height - 1) + u64::from(width) * 4;
    assert!(src.len() as u64 >= required, "source does not cover the frame");
    let (sy, su, sv) = dst.strides();
    let (y, u, v) = dst.data_mut();
    // SAFETY: The source was bounded above; the owned I420 buffer supplies valid
    // planes and strides for exactly these dimensions. libyuv finishes synchronously.
    unsafe {
        yuv_sys::ffi::argb_to_i420_matrix(
            src.as_ptr(),
            stride as i32,
            y.as_mut_ptr(),
            sy as i32,
            u.as_mut_ptr(),
            su as i32,
            v.as_mut_ptr(),
            sv as i32,
            width as i32,
            height as i32,
            matrix as u8,
        )
        .expect("validated RGB to I420 conversion");
    }
}

/// Converts BGRA bytes into full-resolution 8-bit planes without chroma subsampling.
///
/// `Some(matrix)` produces YUV 4:4:4. `None` copies RGB into G/B/R planes, which
/// require RGB/identity colour metadata and full range on the source and codec.
/// Panics if the source stride or slice cannot cover the destination dimensions.
pub fn argb_to_i444(
    src: &[u8],
    stride: u32,
    dst: &mut crate::video_frame::I444Buffer,
    matrix: Option<YuvMatrix>,
) {
    let (width, height) = (dst.width(), dst.height());
    assert!(width > 0 && height > 0 && width <= i32::MAX as u32 && height <= i32::MAX as u32);
    assert!(stride <= i32::MAX as u32 && u64::from(stride) >= u64::from(width) * 4);
    let required = u64::from(stride) * u64::from(height - 1) + u64::from(width) * 4;
    assert!(src.len() as u64 >= required, "source does not cover the frame");
    let (sy, su, sv) = dst.strides();
    let (y, u, v) = dst.data_mut();
    // SAFETY: The source is bounded above. The owned buffer supplies valid full-size
    // planes and strides; libyuv borrows them only for this synchronous conversion.
    unsafe {
        yuv_sys::ffi::argb_to_i444_matrix(
            src.as_ptr(),
            stride as i32,
            y.as_mut_ptr(),
            sy as i32,
            u.as_mut_ptr(),
            su as i32,
            v.as_mut_ptr(),
            sv as i32,
            width as i32,
            height as i32,
            matrix.map_or(4, |m| m as u8),
        )
        .expect("validated full-chroma conversion");
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::{argb_to_i420_with_matrix, argb_to_i444, YuvMatrix};
    use crate::video_frame::{I420Buffer, I444Buffer};

    #[test]
    fn full_chroma_keeps_adjacent_colors_and_rgb_planes_exact_with_padded_rows() {
        let pixels =
            [0, 0, 255, 255, 255, 0, 0, 255, 99, 99, 99, 99, 0, 255, 0, 255, 255, 255, 255, 255];
        let mut output = I444Buffer::new(2, 2);
        argb_to_i444(&pixels, 12, &mut output, None);
        let mut packed = [99; 24];
        super::gbr_to_argb(&output, &mut packed, 16);
        assert_eq!(&packed[..8], &pixels[..8]);
        assert_eq!(&packed[16..], &pixels[12..]);
        assert_eq!(&packed[8..16], &[99; 8]);
        assert_eq!(
            output.data(),
            (&[0, 0, 255, 255][..], &[0, 255, 0, 255][..], &[255, 0, 0, 255][..])
        );
        for matrix in [
            YuvMatrix::Bt601Limited,
            YuvMatrix::Bt601Full,
            YuvMatrix::Bt709Limited,
            YuvMatrix::Bt709Full,
        ] {
            argb_to_i444(&pixels, 12, &mut output, Some(matrix));
            let (y, u, v) = output.data();
            // A one-pixel primary must agree with a uniform 2x2 reference in
            // every matrix, without averaging the neighboring primary's chroma.
            for (i, bgra) in [
                pixels[0..4].to_vec(),
                pixels[4..8].to_vec(),
                pixels[12..16].to_vec(),
                pixels[16..20].to_vec(),
            ]
            .iter()
            .enumerate()
            {
                let mut reference = I420Buffer::new(2, 2);
                argb_to_i420_with_matrix(&bgra.repeat(4), 8, &mut reference, matrix);
                let (ry, ru, rv) = reference.data();
                for (actual, expected) in [y[i], u[i], v[i]].into_iter().zip([ry[0], ru[0], rv[0]])
                {
                    assert!(
                        (i16::from(actual) - i16::from(expected)).abs() <= 2,
                        "{matrix:?}, pixel {i}: actual {actual}, expected {expected}"
                    );
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "source does not cover the frame")]
    fn full_chroma_rejects_truncated_input_before_ffi() {
        argb_to_i444(&[0; 15], 8, &mut I444Buffer::new(2, 2), None);
    }

    #[test]
    #[should_panic(expected = "destination does not cover the frame")]
    fn full_chroma_rejects_truncated_packed_output_before_ffi() {
        super::gbr_to_argb(&I444Buffer::new(2, 2), &mut [0; 15], 8);
    }

    #[test]
    fn scaling_full_chroma_preserves_three_full_resolution_planes() {
        let mut output = I444Buffer::new(4, 4);
        argb_to_i444(&[11, 23, 37, 255].repeat(16), 16, &mut output, None);
        let scaled = output.scale(2, 2);
        assert_eq!(scaled.data(), (&[23; 4][..], &[11; 4][..], &[37; 4][..]));
    }

    #[test]
    fn color_conversion_accepts_row_padding_without_reading_a_final_padding_row() {
        let mut pixels = vec![0; 24]; // two 8-byte rows, with 8 bytes of padding between them
        pixels[..8].fill(255);
        pixels[16..].fill(255);
        let mut output = I420Buffer::new(2, 2);
        argb_to_i420_with_matrix(&pixels, 16, &mut output, YuvMatrix::Bt709Full);
        assert!(output.data().0.iter().all(|value| *value == 255));
    }

    #[test]
    #[should_panic(expected = "source does not cover the frame")]
    fn color_conversion_rejects_a_truncated_source_before_entering_ffi() {
        let mut output = I420Buffer::new(2, 2);
        argb_to_i420_with_matrix(&[0; 15], 8, &mut output, YuvMatrix::Bt709Full);
    }
}

/// Packs G/B/R planes held in an I444 buffer into opaque BGRA bytes (libyuv ARGB).
/// No YUV conversion is performed. Panics when the output stride/slice is too small.
pub fn gbr_to_argb(src: &crate::video_frame::I444Buffer, dst: &mut [u8], stride: u32) {
    let (width, height) = (src.width(), src.height());
    assert!(width > 0 && height > 0 && width <= i32::MAX as u32 && height <= i32::MAX as u32);
    assert!(stride <= i32::MAX as u32 && u64::from(stride) >= u64::from(width) * 4);
    let required = u64::from(stride) * u64::from(height - 1) + u64::from(width) * 4;
    assert!(dst.len() as u64 >= required, "destination does not cover the frame");
    let (sg, sb, sr) = src.strides();
    let (g, b, r) = src.data();
    // SAFETY: owned planes cover the input; output bounds are checked above.
    // libyuv borrows all buffers only for this synchronous copy.
    unsafe {
        yuv_sys::ffi::gbr_to_argb(
            g.as_ptr(),
            sg as i32,
            b.as_ptr(),
            sb as i32,
            r.as_ptr(),
            sr as i32,
            dst.as_mut_ptr(),
            stride as i32,
            width as i32,
            height as i32,
        );
    }
}

fn argb_assert_safety(src: &[u8], src_stride: u32, _width: i32, height: i32) {
    let height_abs = height.unsigned_abs();
    let min = (src_stride * height_abs) as usize;
    assert!(src.len() >= min, "src isn't large enough");
}

fn i420_assert_safety(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    _width: i32,
    height: i32,
) {
    let height_abs = height.unsigned_abs();
    let chroma_height = (height_abs + 1) / 2;
    let min_y = (src_stride_y * height_abs) as usize;
    let min_u = (src_stride_u * chroma_height) as usize;
    let min_v = (src_stride_v * chroma_height) as usize;

    assert!(src_y.len() >= min_y, "src_y isn't large enough");
    assert!(src_u.len() >= min_u, "src_u isn't large enough");
    assert!(src_v.len() >= min_v, "src_v isn't large enough");
}

fn nv12_assert_safety(
    src_y: &[u8],
    src_stride_y: u32,
    src_uv: &[u8],
    src_stride_uv: u32,
    _width: i32,
    height: i32,
) {
    let height_abs = height.unsigned_abs();
    let chroma_height = (height_abs + 1) / 2;

    let min_y = (src_stride_y * height_abs) as usize;
    let min_uv = (src_stride_uv * chroma_height) as usize;

    assert!(src_y.len() >= min_y, "src_y isn't large enough");
    assert!(src_uv.len() >= min_uv, "src_uv isn't large enough");
}

fn i444_assert_safety(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    _width: i32,
    height: i32,
) {
    let height_abs = height.unsigned_abs();
    let min_y = (src_stride_y * height_abs) as usize;
    let min_u = (src_stride_u * height_abs) as usize;
    let min_v = (src_stride_v * height_abs) as usize;

    assert!(src_y.len() >= min_y, "src_y isn't large enough");
    assert!(src_u.len() >= min_u, "src_u isn't large enough");
    assert!(src_v.len() >= min_v, "src_v isn't large enough");
}

fn i422_assert_safety(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    _width: i32,
    height: i32,
) {
    let height_abs = height.unsigned_abs();
    let min_y = (src_stride_y * height_abs) as usize;
    let min_u = (src_stride_u * height_abs) as usize;
    let min_v = (src_stride_v * height_abs) as usize;

    assert!(src_y.len() >= min_y, "src_y isn't large enough");
    assert!(src_u.len() >= min_u, "src_u isn't large enough");
    assert!(src_v.len() >= min_v, "src_v isn't large enough");
}

fn i010_assert_safety(
    src_y: &[u16],
    src_stride_y: u32,
    src_u: &[u16],
    src_stride_u: u32,
    src_v: &[u16],
    src_stride_v: u32,
    _width: i32,
    height: i32,
) {
    let height_abs: u32 = height.unsigned_abs();
    let chroma_height = height_abs / 2;
    let min_y = (src_stride_y * height_abs) as usize / 2;
    let min_u = (src_stride_u * chroma_height) as usize / 2;
    let min_v = (src_stride_v * chroma_height) as usize / 2;

    assert!(src_y.len() >= min_y, "src_y isn't large enough");
    assert!(src_u.len() >= min_u, "src_u isn't large enough");
    assert!(src_v.len() >= min_v, "src_v isn't large enough");
}

macro_rules! i420_to_rgba {
    ($x:ident) => {
        pub fn $x(
            src_y: &[u8],
            src_stride_y: u32,
            src_u: &[u8],
            src_stride_u: u32,
            src_v: &[u8],
            src_stride_v: u32,
            dst: &mut [u8],
            dst_stride: u32,
            width: i32,
            height: i32,
        ) {
            i420_assert_safety(
                src_y,
                src_stride_y,
                src_u,
                src_stride_u,
                src_v,
                src_stride_v,
                width,
                height,
            );
            argb_assert_safety(dst, dst_stride, width, height);

            unsafe {
                yuv_sys::ffi::$x(
                    src_y.as_ptr(),
                    src_stride_y as i32,
                    src_u.as_ptr(),
                    src_stride_u as i32,
                    src_v.as_ptr(),
                    src_stride_v as i32,
                    dst.as_mut_ptr(),
                    dst_stride as i32,
                    width,
                    height,
                )
                .unwrap();
            }
        }
    };
}

macro_rules! rgba_to_i420 {
    ($x:ident) => {
        pub fn $x(
            src_argb: &[u8],
            src_stride_argb: u32,
            dst_y: &mut [u8],
            dst_stride_y: u32,
            dst_u: &mut [u8],
            dst_stride_u: u32,
            dst_v: &mut [u8],
            dst_stride_v: u32,
            width: i32,
            height: i32,
        ) {
            i420_assert_safety(
                dst_y,
                dst_stride_y,
                dst_u,
                dst_stride_u,
                dst_v,
                dst_stride_v,
                width,
                height,
            );
            argb_assert_safety(src_argb, src_stride_argb, width, height);

            unsafe {
                yuv_sys::ffi::$x(
                    src_argb.as_ptr(),
                    src_stride_argb as i32,
                    dst_y.as_mut_ptr(),
                    dst_stride_y as i32,
                    dst_u.as_mut_ptr(),
                    dst_stride_u as i32,
                    dst_v.as_mut_ptr(),
                    dst_stride_v as i32,
                    width,
                    height,
                )
                .unwrap();
            }
        }
    };
}

pub fn argb_to_rgb24(
    src_argb: &[u8],
    src_stride_argb: u32,
    dst_rgb24: &mut [u8],
    dst_stride_rgb24: u32,
    width: i32,
    height: i32,
) {
    argb_assert_safety(src_argb, src_stride_argb, width, height);
    argb_assert_safety(dst_rgb24, dst_stride_rgb24, width, height);

    unsafe {
        yuv_sys::ffi::argb_to_rgb24(
            src_argb.as_ptr(),
            src_stride_argb as i32,
            dst_rgb24.as_mut_ptr(),
            dst_stride_rgb24 as i32,
            width,
            height,
        )
        .unwrap();
    }
}

// I420 <> RGB conversion
rgba_to_i420!(argb_to_i420);
rgba_to_i420!(abgr_to_i420);

i420_to_rgba!(i420_to_argb);
i420_to_rgba!(i420_to_bgra);
i420_to_rgba!(i420_to_abgr);
i420_to_rgba!(i420_to_rgba);

pub fn i420_to_nv12(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_uv: &mut [u8],
    dst_stride_uv: u32,
    width: i32,
    height: i32,
) {
    i420_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    nv12_assert_safety(dst_y, dst_stride_y, dst_uv, dst_stride_uv, width, height);

    unsafe {
        yuv_sys::ffi::i420_to_nv12(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_uv.as_mut_ptr(),
            dst_stride_uv as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn nv12_to_i420(
    src_y: &[u8],
    src_stride_y: u32,
    src_uv: &[u8],
    src_stride_uv: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_u: &mut [u8],
    dst_stride_u: u32,
    dst_v: &mut [u8],
    dst_stride_v: u32,
    width: i32,
    height: i32,
) {
    nv12_assert_safety(src_y, src_stride_y, src_uv, src_stride_uv, width, height);
    i420_assert_safety(
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width,
        height,
    );

    unsafe {
        yuv_sys::ffi::nv12_to_i420(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_uv.as_ptr(),
            src_stride_uv as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_u.as_mut_ptr(),
            dst_stride_u as i32,
            dst_v.as_mut_ptr(),
            dst_stride_v as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn i444_to_i420(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_u: &mut [u8],
    dst_stride_u: u32,
    dst_v: &mut [u8],
    dst_stride_v: u32,
    width: i32,
    height: i32,
) {
    i444_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    i420_assert_safety(
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width,
        height,
    );

    unsafe {
        yuv_sys::ffi::i444_to_i420(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_u.as_mut_ptr(),
            dst_stride_u as i32,
            dst_v.as_mut_ptr(),
            dst_stride_v as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn i422_to_i420(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_u: &mut [u8],
    dst_stride_u: u32,
    dst_v: &mut [u8],
    dst_stride_v: u32,
    width: i32,
    height: i32,
) {
    i422_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    i420_assert_safety(
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width,
        height,
    );

    unsafe {
        yuv_sys::ffi::i422_to_i420(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_u.as_mut_ptr(),
            dst_stride_u as i32,
            dst_v.as_mut_ptr(),
            dst_stride_v as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn i010_to_i420(
    src_y: &[u16],
    src_stride_y: u32,
    src_u: &[u16],
    src_stride_u: u32,
    src_v: &[u16],
    src_stride_v: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_u: &mut [u8],
    dst_stride_u: u32,
    dst_v: &mut [u8],
    dst_stride_v: u32,
    width: i32,
    height: i32,
) {
    i010_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    i420_assert_safety(
        dst_y,
        dst_stride_y,
        dst_u,
        dst_stride_u,
        dst_v,
        dst_stride_v,
        width,
        height,
    );

    unsafe {
        yuv_sys::ffi::i010_to_i420(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_u.as_mut_ptr(),
            dst_stride_u as i32,
            dst_v.as_mut_ptr(),
            dst_stride_v as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn nv12_to_argb(
    src_y: &[u8],
    src_stride_y: u32,
    src_uv: &[u8],
    src_stride_uv: u32,
    dst_argb: &mut [u8],
    dst_stride_argb: u32,
    width: i32,
    height: i32,
) {
    nv12_assert_safety(src_y, src_stride_y, src_uv, src_stride_uv, width, height);
    argb_assert_safety(dst_argb, dst_stride_argb, width, height);

    unsafe {
        yuv_sys::ffi::nv12_to_argb(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_uv.as_ptr(),
            src_stride_uv as i32,
            dst_argb.as_mut_ptr(),
            dst_stride_argb as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn nv12_to_abgr(
    src_y: &[u8],
    src_stride_y: u32,
    src_uv: &[u8],
    src_stride_uv: u32,
    dst_abgr: &mut [u8],
    dst_stride_abgr: u32,
    width: i32,
    height: i32,
) {
    nv12_assert_safety(src_y, src_stride_y, src_uv, src_stride_uv, width, height);
    argb_assert_safety(dst_abgr, dst_stride_abgr, width, height);

    unsafe {
        yuv_sys::ffi::nv12_to_abgr(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_uv.as_ptr(),
            src_stride_uv as i32,
            dst_abgr.as_mut_ptr(),
            dst_stride_abgr as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn i444_to_argb(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_argb: &mut [u8],
    dst_stride_argb: u32,
    width: i32,
    height: i32,
) {
    i444_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    argb_assert_safety(dst_argb, dst_stride_argb, width, height);

    unsafe {
        yuv_sys::ffi::i444_to_argb(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_argb.as_mut_ptr(),
            dst_stride_argb as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn i444_to_abgr(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_abgr: &mut [u8],
    dst_stride_abgr: u32,
    width: i32,
    height: i32,
) {
    i444_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    argb_assert_safety(dst_abgr, dst_stride_abgr, width, height);

    unsafe {
        yuv_sys::ffi::i444_to_abgr(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_abgr.as_mut_ptr(),
            dst_stride_abgr as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn i422_to_argb(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_argb: &mut [u8],
    dst_stride_argb: u32,
    width: i32,
    height: i32,
) {
    i422_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    argb_assert_safety(dst_argb, dst_stride_argb, width, height);

    unsafe {
        yuv_sys::ffi::i422_to_argb(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_argb.as_mut_ptr(),
            dst_stride_argb as i32,
            width,
            height,
        )
        .unwrap();
    }
}

pub fn i422_to_abgr(
    src_y: &[u8],
    src_stride_y: u32,
    src_u: &[u8],
    src_stride_u: u32,
    src_v: &[u8],
    src_stride_v: u32,
    dst_abgr: &mut [u8],
    dst_stride_abgr: u32,
    width: i32,
    height: i32,
) {
    i422_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    argb_assert_safety(dst_abgr, dst_stride_abgr, width, height);

    unsafe {
        yuv_sys::ffi::i422_to_abgr(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_abgr.as_mut_ptr(),
            dst_stride_abgr as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn i010_to_argb(
    src_y: &[u16],
    src_stride_y: u32,
    src_u: &[u16],
    src_stride_u: u32,
    src_v: &[u16],
    src_stride_v: u32,
    dst_argb: &mut [u8],
    dst_stride_argb: u32,
    width: i32,
    height: i32,
) {
    i010_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    argb_assert_safety(dst_argb, dst_stride_argb, width, height);

    unsafe {
        yuv_sys::ffi::i010_to_argb(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_argb.as_mut_ptr(),
            dst_stride_argb as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn i010_to_abgr(
    src_y: &[u16],
    src_stride_y: u32,
    src_u: &[u16],
    src_stride_u: u32,
    src_v: &[u16],
    src_stride_v: u32,
    dst_abgr: &mut [u8],
    dst_stride_abgr: u32,
    width: i32,
    height: i32,
) {
    i010_assert_safety(
        src_y,
        src_stride_y,
        src_u,
        src_stride_u,
        src_v,
        src_stride_v,
        width,
        height,
    );
    argb_assert_safety(dst_abgr, dst_stride_abgr, width, height);

    unsafe {
        yuv_sys::ffi::i010_to_abgr(
            src_y.as_ptr(),
            src_stride_y as i32,
            src_u.as_ptr(),
            src_stride_u as i32,
            src_v.as_ptr(),
            src_stride_v as i32,
            dst_abgr.as_mut_ptr(),
            dst_stride_abgr as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn abgr_to_nv12(
    src_abgr: &[u8],
    src_stride_abgr: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_uv: &mut [u8],
    dst_stride_uv: u32,
    width: i32,
    height: i32,
) {
    argb_assert_safety(src_abgr, src_stride_abgr, width, height);
    nv12_assert_safety(dst_y, dst_stride_y, dst_uv, dst_stride_uv, width, height);

    unsafe {
        yuv_sys::ffi::abgr_to_nv12(
            src_abgr.as_ptr(),
            src_stride_abgr as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_uv.as_mut_ptr(),
            dst_stride_uv as i32,
            width,
            height,
        )
        .unwrap()
    }
}

pub fn argb_to_nv12(
    src_argb: &[u8],
    src_stride_argb: u32,
    dst_y: &mut [u8],
    dst_stride_y: u32,
    dst_uv: &mut [u8],
    dst_stride_uv: u32,
    width: i32,
    height: i32,
) {
    argb_assert_safety(src_argb, src_stride_argb, width, height);
    nv12_assert_safety(dst_y, dst_stride_y, dst_uv, dst_stride_uv, width, height);

    unsafe {
        yuv_sys::ffi::argb_to_nv12(
            src_argb.as_ptr(),
            src_stride_argb as i32,
            dst_y.as_mut_ptr(),
            dst_stride_y as i32,
            dst_uv.as_mut_ptr(),
            dst_stride_uv as i32,
            width,
            height,
        )
        .unwrap()
    }
}
