use std::collections::HashSet;

use arcnix_core::{FileEntry, TransferJob, TransferStatus, new_id};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    Size,
    Modified,
}
pub fn visible(entries: &[FileEntry], query: &str, sort: Sort, descending: bool) -> Vec<FileEntry> {
    let query = query.to_lowercase();
    let mut rows: Vec<_> = entries
        .iter()
        .filter(|f| f.name.to_lowercase().contains(&query))
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let order = match sort {
                Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                Sort::Size => a.size.cmp(&b.size),
                Sort::Modified => a.modified.cmp(&b.modified),
            };
            let order = order.then_with(|| a.name.cmp(&b.name));
            if descending { order.reverse() } else { order }
        })
    });
    rows
}
pub fn retry(
    jobs: &mut [TransferJob],
    active: &HashSet<String>,
    server: Option<&str>,
    id: Option<&str>,
) {
    for job in jobs {
        if Some(job.server_id.as_str()) == server
            && id.is_none_or(|id| id == job.id)
            && !active.contains(&job.id)
            && matches!(
                job.status,
                TransferStatus::Cancelled | TransferStatus::Failed(_)
            )
        {
            job.id = new_id();
            job.status = TransferStatus::Queued;
            job.transferred_bytes = 0;
        }
    }
}
pub fn queue_visible(status: &TransferStatus, filter: u8) -> bool {
    match filter {
        1 => matches!(status, TransferStatus::Queued | TransferStatus::Running),
        2 => matches!(
            status,
            TransferStatus::Failed(_) | TransferStatus::Cancelled
        ),
        3 => *status == TransferStatus::Completed,
        _ => true,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(name: &str, dir: bool, size: u64) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: name.into(),
            is_dir: dir,
            is_symlink: false,
            size,
            modified: None,
            permissions: None,
        }
    }
    #[test]
    fn directories_stay_first_in_reverse_size_sort() {
        let rows = visible(
            &[
                entry("z", false, 20),
                entry("a", false, 10),
                entry("folder", true, 0),
            ],
            "",
            Sort::Size,
            true,
        );
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["folder", "z", "a"]
        );
    }
    #[test]
    fn filter_is_case_insensitive_and_preserves_unicode() {
        let rows = visible(
            &[entry("中文.TXT", false, 1), entry("abc", false, 1)],
            ".txt",
            Sort::Name,
            false,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "中文.TXT");
    }
    #[test]
    fn retry_changes_identity_and_excludes_in_flight_or_other_server() {
        let make = |id: &str, server: &str| TransferJob {
            id: id.into(),
            server_id: server.into(),
            direction: arcnix_core::TransferDirection::Upload,
            local_path: "file".into(),
            remote_path: "/file".into(),
            total_bytes: 10,
            transferred_bytes: 5,
            status: TransferStatus::Cancelled,
        };
        let mut jobs = vec![make("a", "one"), make("b", "one"), make("c", "two")];
        retry(&mut jobs, &HashSet::from(["b".into()]), Some("one"), None);
        assert_ne!(jobs[0].id, "a");
        assert_eq!(jobs[0].status, TransferStatus::Queued);
        assert_eq!(jobs[1].id, "b");
        assert_eq!(jobs[2].id, "c");
    }
}
