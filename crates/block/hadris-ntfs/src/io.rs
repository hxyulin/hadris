//! I/O trait re-exports and utilities for NTFS.

io_transform! {

pub use super::super::{Read, ReadExt, Seek, Error, ErrorKind, SeekFrom, Parsable};
pub use super::super::IoResult;

use crate::attr::DataRun;
use crate::error::{NtfsError, Result};

/// Read a contiguous byte range from a series of data runs.
///
/// This is the fundamental I/O primitive for NTFS: it translates a logical
/// byte offset within a data stream (described by `runs`) into one or more
/// physical disk reads.
pub(crate) async fn read_data_runs<DATA: Read + Seek>(
    data: &mut DATA,
    runs: &[DataRun],
    offset: u64,
    buf: &mut [u8],
    cluster_size: u64,
) -> Result<()> {
    if cluster_size == 0 {
        return Err(NtfsError::InvalidDataRun);
    }
    let end = offset
        .checked_add(buf.len() as u64)
        .ok_or(NtfsError::InvalidDataRun)?;
    let mut run_start: u64 = 0;
    let mut filled: usize = 0;

    for run in runs {
        let run_bytes = run
            .length
            .checked_mul(cluster_size)
            .ok_or(NtfsError::InvalidDataRun)?;
        let run_end = run_start
            .checked_add(run_bytes)
            .ok_or(NtfsError::InvalidDataRun)?;

        if run_end <= offset {
            run_start = run_end;
            continue;
        }
        if run_start >= end {
            break;
        }

        let read_start = offset.max(run_start);
        let read_end = end.min(run_end);
        let bytes_to_read = (read_end - read_start) as usize;

        if run.lcn < 0 {
            // Sparse run — zero-fill
            for b in &mut buf[filled..filled + bytes_to_read] {
                *b = 0;
            }
        } else {
            let offset_in_run = read_start - run_start;
            let disk_pos = (run.lcn as u64)
                .checked_mul(cluster_size)
                .and_then(|start| start.checked_add(offset_in_run))
                .ok_or(NtfsError::InvalidDataRun)?;
            data.seek(SeekFrom::Start(disk_pos)).await?;
            data.read_exact(&mut buf[filled..filled + bytes_to_read]).await?;
        }

        filled += bytes_to_read;
        run_start = run_end;
    }

    if filled < buf.len() {
        return Err(NtfsError::UnexpectedEndOfData);
    }
    Ok(())
}

} // end io_transform!
