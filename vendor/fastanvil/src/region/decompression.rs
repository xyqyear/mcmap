use std::io;
use std::ptr::NonNull;

use libdeflate_sys::{
    libdeflate_alloc_decompressor, libdeflate_decompressor, libdeflate_free_decompressor,
    libdeflate_gzip_decompress_ex,
    libdeflate_result_LIBDEFLATE_INSUFFICIENT_SPACE as INSUFFICIENT_SPACE,
    libdeflate_result_LIBDEFLATE_SUCCESS as SUCCESS, libdeflate_zlib_decompress_ex,
};

use super::CompressionScheme;

#[derive(Default)]
pub(super) struct Decoder {
    inner: Option<NonNull<libdeflate_decompressor>>,
}

impl Clone for Decoder {
    fn clone(&self) -> Self {
        Self::default()
    }
}

// Decoder calls require exclusive access; clones never share native state.
unsafe impl Send for Decoder {}
unsafe impl Sync for Decoder {}

impl Decoder {
    pub(super) fn decode(
        &mut self,
        scheme: &CompressionScheme,
        input: &[u8],
        output: &mut Vec<u8>,
    ) -> io::Result<bool> {
        // The streaming decoder owns validation of nonstandard declared window sizes.
        if matches!(scheme, CompressionScheme::Zlib)
            && input.first().is_some_and(|cmf| cmf >> 4 != 7)
        {
            return Ok(false);
        }
        // Optional gzip headers retain the streaming decoder's validation and size limits.
        if matches!(scheme, CompressionScheme::Gzip)
            && input.get(3).is_some_and(|flags| flags & 0x1e != 0)
        {
            return Ok(false);
        }
        let decoder = match self.inner {
            Some(decoder) => decoder,
            None => {
                let decoder = NonNull::new(unsafe { libdeflate_alloc_decompressor() })
                    .ok_or_else(|| io::Error::other("could not allocate decompressor"))?;
                self.inner = Some(decoder);
                decoder
            }
        };
        output.clear();
        if output.capacity() == 0 {
            output.try_reserve(64 * 1024).map_err(io::Error::other)?;
        }
        loop {
            let mut consumed = 0;
            let mut written = 0;
            let decode = match scheme {
                CompressionScheme::Zlib => libdeflate_zlib_decompress_ex,
                CompressionScheme::Gzip => libdeflate_gzip_decompress_ex,
                _ => return Ok(false),
            };
            // The native decoder writes at most capacity bytes into this exclusive buffer.
            let result = unsafe {
                decode(
                    decoder.as_ptr(),
                    input.as_ptr().cast(),
                    input.len(),
                    output.as_mut_ptr().cast(),
                    output.capacity(),
                    &mut consumed,
                    &mut written,
                )
            };
            match result {
                SUCCESS if consumed == input.len() && written <= output.capacity() => {
                    // Only a successful decode makes the reported bytes initialized.
                    unsafe { output.set_len(written) };
                    return Ok(true);
                }
                INSUFFICIENT_SPACE => {
                    let capacity = output
                        .capacity()
                        .checked_mul(2)
                        .ok_or_else(|| io::Error::other("decompressed chunk is too large"))?;
                    output.try_reserve(capacity).map_err(io::Error::other)?;
                }
                _ => return Ok(false),
            }
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if let Some(decoder) = self.inner {
            unsafe { libdeflate_free_decompressor(decoder.as_ptr()) };
        }
    }
}
