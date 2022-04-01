//! Support for JPG image decoding.
use snafu::{OptionExt, ResultExt};

use super::{CustomMessageSnafu, CustomSnafu, DecodeError, MissingAttributeSnafu};
use crate::adapters::{DecodeResult, PixelDataObject, PixelRWAdapter};
use jpeg_decoder::Decoder;
use std::io::Cursor;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JPEGAdapter;

impl JPEGAdapter {
    fn encoded_frames(&self, src: &dyn PixelDataObject) -> Result<Vec<Vec<u8>>, DecodeError> {
        // Embedded jpegs can span multiple fragments
        // Create 1:1 mapping between frame and fragment data
        // https://dicom.nema.org/dicom/2013/output/chtml/part05/sect_8.2.html
        let nr_frames = src.number_of_frames().unwrap_or(1) as usize;
        let mut frame_to_fragments: Vec<Vec<u8>> = vec![Vec::new(); nr_frames];
        let fragments = src
            .raw_pixel_data()
            .context(CustomMessageSnafu {
                message: "Expected to have raw pixel data available",
            })?
            .fragments;
        {
            let mut current_frame = 0;
            for fragment in &fragments {
                let mut decoder = Decoder::new(Cursor::new(fragment));
                let info = decoder.read_info();
                let is_new_frame = info.is_ok();
                if is_new_frame {
                    frame_to_fragments[current_frame].extend_from_slice(fragment);
                    current_frame += 1;
                } else if current_frame > 0 {
                    // Not the start of a new frame
                    // try to append to last known frame if already created
                    frame_to_fragments[current_frame - 1].extend_from_slice(fragment);
                } else {
                    // when reading the initial frame we got an error from the decoder
                    info.map_err(|e| Box::new(e) as Box<_>)
                        .context(CustomSnafu)?;
                }
            }
            if current_frame != nr_frames {
                return CustomMessageSnafu {
                    message: "Could not extract expected number of frames from fragments",
                }
                .fail();
            }
        }
        Ok(frame_to_fragments)
    }
}

impl PixelRWAdapter for JPEGAdapter {
    /// Decode DICOM image data with jpeg encoding.

    fn decode(&self, src: &dyn PixelDataObject, dst: &mut Vec<u8>) -> DecodeResult<()> {
        let cols = src
            .cols()
            .context(MissingAttributeSnafu { name: "Columns" })?;
        let rows = src.rows().context(MissingAttributeSnafu { name: "Rows" })?;
        let samples_per_pixel = src.samples_per_pixel().context(MissingAttributeSnafu {
            name: "SamplesPerPixel",
        })?;
        let bits_allocated = src.bits_allocated().context(MissingAttributeSnafu {
            name: "BitsAllocated",
        })?;

        if bits_allocated != 8 && bits_allocated != 16 {
            return CustomMessageSnafu {
                message: "BitsAllocated other than 8 or 16 is not supported",
            }
            .fail();
        }

        let nr_frames = src.number_of_frames().unwrap_or(1) as usize;
        let bytes_per_sample = bits_allocated / 8;
        let encoded_frames = self.encoded_frames(src)?;

        // `stride` it the total number of bytes for each sample plane
        let stride: usize = (bytes_per_sample as usize * cols as usize * rows as usize).into();
        dst.resize((samples_per_pixel as usize * stride) * nr_frames, 0);

        let mut offset = 0;
        for frame in encoded_frames {
            let mut decoder = Decoder::new(Cursor::new(frame));

            let decoded = decoder
                .decode()
                .map_err(|e| Box::new(e) as Box<_>)
                .context(CustomSnafu)?;

            let decoded_len = decoded.len();
            dst[offset..(offset + decoded_len)].copy_from_slice(&decoded);
            offset += decoded_len
        }
        Ok(())
    }
}
