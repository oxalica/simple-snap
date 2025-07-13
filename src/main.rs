use std::{
    num::NonZero,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use palc::{Args, Parser, Subcommand};

mod ioctl;

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Create a BTRFS snapshot for a subvolume.
    ///
    /// Create a snapshot under TARGET_DIR for subvolume SOURCE. The snapshot is
    /// named as PREFIX (empty if omitted) joined with current RFC3339 timestamp
    /// with offset of local time zone.
    ///
    /// This behaves like `btrfs subvolume snapshot` with sugar, but does not
    /// depends on btrfs-progs.
    Snapshot {
        /// The directory to store created snapshots.
        ///
        /// It must exist and is not a symlink.
        #[arg(long, short = 'd')]
        target_dir: PathBuf,
        /// The prefix for snapshot names.
        ///
        /// If omitted, an empty string is used.
        #[arg(long, default_value_t)]
        prefix: String,
        /// The source subvolume to create snapshot for.
        #[arg(long, short)]
        source: PathBuf,

        /// Only create the snapshot if there is any change since the latest
        /// snapshot, else do nothing.
        ///
        /// Change detection is based on the equality of subvolume generations.
        #[arg(long)]
        skip_if_unchanged: bool,

        /// Print the actions that would be done without doing them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Prune BTRFS snapshots according to specific retention policies.
    ///
    /// All directories under TARGET_DIR prefixed by PREFIX (empty if omitted)
    /// are subjects to prune. They must be be parsable as a jiff timestamp
    /// after stripping PREFIX. The timestamp is expected to be in RFC3339 format.
    /// See all supported formats in:
    /// <https://docs.rs/jiff/0.2.15/jiff/fmt/index.html#support-for-fromstr-and-display>
    ///
    /// One or more policies must be specified. Snapshots will be kept if they
    /// are covered by any policy. In other words, only snapshots that are not
    /// covered by any policy will be deleted.
    ///
    /// Note 1: Subvolume deletion requires sufficient permissions, and does not
    /// involve a full transaction commit. We do not wait (`sync`) for
    /// transaction completion before exit either.
    /// See details in:
    /// <https://btrfs.readthedocs.io/en/latest/btrfs-subvolume.html#subcommand>
    ///
    /// Note 2: All calendar units, eg. `--keep-daily`, use local timezone.
    /// The calendar arithmetic calculates on the specified units in local
    /// timezone, that is, "1 month" before "2025-03-01" is always "2025-02-01"
    /// regardless how many days there are in Feb 2025.
    /// If you intend to use UTC, set the environment variable `TZ=Etc/UTC`.
    Prune {
        /// The directory containing snapshots to prune.
        ///
        /// It must exist and is not a symlink.
        #[arg(long, short = 'd')]
        target_dir: PathBuf,
        /// The prefix of snapshots to prune.
        ///
        /// If omitted, an empty string is used.
        #[arg(long, default_value_t)]
        prefix: String,
        #[command(flatten)]
        policy: RetentionPolicy,

        /// Print the actions that would be done without doing them.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Args)]
struct RetentionPolicy {
    /// Keep the N last (most recent) snapshots.
    #[arg(long, value_name = "N")]
    keep_last: Option<NonZero<u16>>,
    /// Keep all snapshots having a timestamp within the specified span before current time.
    ///
    /// SPAN supports ISO8601 `P2m10dT2h30m` or jiff's friendly format `3d 4h 59m`.
    /// See details in <https://docs.rs/jiff/0.2.15/jiff/struct.Span.html#parsing-and-printing>.
    #[arg(long, value_name = "SPAN")]
    keep_within: Option<jiff::Span>,

    /// For the last N hours which have one or more snapshots, keep only the most recent one for each hour.
    #[arg(long, value_name = "N")]
    keep_hourly: Option<NonZero<u16>>,
    /// For the last N days which have one or more snapshots, keep only the most recent one for each day.
    #[arg(long, value_name = "N")]
    keep_daily: Option<NonZero<u16>>,
    /// For the last N weeks which have one or more snapshots, keep only the most recent one for each week.
    ///
    /// Note: A week starts at Monday 00:00:00, following the definition of restic.
    #[arg(long, value_name = "N")]
    keep_weekly: Option<NonZero<u16>>,
    /// For the last N months which have one or more snapshots, keep only the most recent one for each month.
    #[arg(long, value_name = "N")]
    keep_monthly: Option<NonZero<u16>>,
    /// For the last N years which have one or more snapshots, keep only the most recent one for each year.
    #[arg(long, value_name = "N")]
    keep_yearly: Option<NonZero<u16>>,
}

impl RetentionPolicy {
    // NB. This is NOT `is_empty` so that forgetting a field does less harm.
    fn is_valid(&self) -> bool {
        self.keep_last.is_some()
            || self.keep_within.is_some()
            || self.keep_hourly.is_some()
            || self.keep_daily.is_some()
            || self.keep_weekly.is_some()
            || self.keep_monthly.is_some()
            || self.keep_yearly.is_some()
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.cmd {
        CliCommand::Snapshot {
            target_dir,
            prefix,
            source,
            skip_if_unchanged,
            dry_run,
        } => run_snapshot(target_dir, prefix, source, *skip_if_unchanged, *dry_run),
        CliCommand::Prune {
            target_dir,
            prefix: name,
            policy,
            dry_run,
        } => run_prune(target_dir, name, policy, *dry_run),
    }
}

fn run_snapshot(
    target_dir: &Path,
    prefix: &str,
    source: &Path,
    skip_if_unchanged: bool,
    dry_run: bool,
) -> Result<()> {
    let target_dir_fd = open_dir(None, target_dir).context("failed to open target directory")?;
    let subvol_fd = open_dir(None, source).context("failed to open subvolume directory")?;

    ensure!(
        ioctl::subvol_getflags(&subvol_fd).is_ok(),
        "{} is not a BTRFS subvolume",
        source.display()
    );

    let now = jiff::Zoned::now();

    let snap_name = format!(
        "{}{}",
        prefix,
        jiff::fmt::temporal::DateTimePrinter::new()
            .timestamp_with_offset_to_string(&now.timestamp(), now.offset())
    );
    let target_path = target_dir.join(&snap_name);

    if skip_if_unchanged
        && let Some(latest_snap) =
            list_snapshots(target_dir_fd.as_fd(), prefix, now.timestamp())?.first()
    {
        let snap_fd = open_dir(Some(target_dir_fd.as_fd()), latest_snap.file_name.as_ref())?;
        let snap_info = ioctl::get_subvol_info(&snap_fd)?;
        let src_info = ioctl::get_subvol_info(&subvol_fd)?;
        // (source UUID, source gen) == (snap parent UUID, snap gen at creation)
        if (src_info.uuid, src_info.generation) == (snap_info.parent_uuid, snap_info.otransid) {
            eprintln!(
                "source {:?} is unchanged from the latest snapshot {:?}, do nothing",
                source.display(),
                latest_snap.file_name,
            );
            return Ok(());
        }
    }

    if dry_run {
        eprintln!(
            "would create snapshot {} for {}",
            target_path.display(),
            source.display()
        );
        eprintln!("exit without action in --dry-run mode");
        return Ok(());
    }

    ioctl::snap_create_v2(&target_dir_fd, &snap_name, subvol_fd, true)
        .context("failed to create snapshot")?;

    eprintln!(
        "created snapshot {} for {}",
        target_path.display(),
        source.display(),
    );

    Ok(())
}

fn run_prune(
    target_dir: &Path,
    prefix: &str,
    policy: &RetentionPolicy,
    dry_run: bool,
) -> Result<()> {
    ensure!(policy.is_valid(), "at least one policy must be provided");
    ensure!(
        policy.keep_within.is_none_or(|dur| dur.is_positive()),
        "--keep-within only accepts a positive duration",
    );

    let now = jiff::Timestamp::now();
    let target_dir_fd = open_dir(None, target_dir).context("failed to open target directory")?;
    let mut snaps = list_snapshots(target_dir_fd.as_fd(), prefix, now)?;

    if snaps.is_empty() {
        eprintln!("no snapshot is found");
        return Ok(());
    }

    if let Some(dur) = policy.keep_within {
        let keep_since = jiff::Timestamp::now() - dur;
        for s in snaps
            .iter_mut()
            .take_while(|s| s.time.timestamp() >= keep_since)
        {
            s.keep_reason.push_str(",last-within");
        }
    }
    if let Some(last) = policy.keep_last {
        for s in snaps.iter_mut().take(last.get().into()) {
            s.keep_reason.push_str(",last-n");
        }
    }

    type RoundFn = fn(&jiff::Zoned) -> Result<jiff::Zoned, jiff::Error>;
    let calendar_policies: &[(_, _, RoundFn)] = &[
        (",hourly", policy.keep_hourly, |t| {
            t.with().minute(0).second(0).subsec_nanosecond(0).build()
        }),
        (",daily", policy.keep_daily, |t| t.start_of_day()),
        (",weekly", policy.keep_weekly, |t| {
            // Round to the next (exclusive) Monday at 00:00:00, treat it as the start of a (next) week.
            // This is compatible with restic.
            t.start_of_day()?
                .nth_weekday(1, jiff::civil::Weekday::Monday)
        }),
        (",yearly", policy.keep_yearly, |t| {
            t.start_of_day()?.first_of_year()
        }),
    ];
    for (msg, cnt, round) in calendar_policies {
        let Some(cnt) = cnt else { continue };
        let mut cnt = cnt.get();

        let mut last = None;
        for s in &mut snaps {
            let rounded = round(&s.time)
                .with_context(|| format!("failed to round {} to unit {:?}", s.time, &msg[1..]))?
                .timestamp();
            if last.replace(rounded) == Some(rounded) {
                continue;
            }
            s.keep_reason.push_str(msg);

            cnt -= 1;
            if cnt == 0 {
                break;
            }
        }
    }

    let mut to_delete = Vec::with_capacity(snaps.len());
    for s in &snaps {
        let action = if s.keep_reason.is_empty() {
            to_delete.push(s.file_name.as_str());
            "!!delete!!"
        } else {
            &s.keep_reason[1..]
        };
        eprintln!("{} {}", s.file_name, action);
    }

    assert!(
        to_delete.len() < snaps.len(),
        "at least one snapshot would be kept",
    );

    eprintln!(
        "---\nwould keep {} of {} snapshots, and delete {} snapshots.",
        snaps.len() - to_delete.len(),
        snaps.len(),
        to_delete.len(),
    );

    if dry_run {
        eprintln!("exit without action in --dry-run mode");
        return Ok(());
    }

    if to_delete.is_empty() {
        eprintln!("nothing to do.");
        return Ok(());
    }

    for file_name in &to_delete {
        ioctl::snap_destroy_v2(&target_dir_fd, file_name).with_context(|| {
            format!(
                "failed to delete subvolume {}",
                target_dir.join(file_name).display(),
            )
        })?;
    }

    eprintln!("deleted {} snapshots (no commit).", to_delete.len());

    Ok(())
}

struct SnapshotInfo {
    file_name: String,
    time: jiff::Zoned,
    /// Why this snapshot should be kept. Empty means to-be-deleted.
    /// Only used in `run_prune`.
    keep_reason: String,
}

/// List all existing snapshots in `target_dir` has `prefix`,
/// sorted by creation time from latest to earliest.
fn list_snapshots(
    target_dir_fd: BorrowedFd<'_>,
    prefix: &str,
    now: jiff::Timestamp,
) -> Result<Vec<SnapshotInfo>> {
    let mut snaps = Vec::new();

    for ent in
        rustix::fs::Dir::read_from(target_dir_fd).context("failed to read target directory")?
    {
        let ent = ent.context("failed to read target directory")?;
        let file_name = ent.file_name();
        // NB. Raw read from `DIR *` reports "." and "..".
        if !ent.file_type().is_dir() || [&b"."[..], b".."].contains(&file_name.to_bytes()) {
            continue;
        }
        let Some(suffix) = file_name.to_bytes().strip_prefix(prefix.as_bytes()) else {
            continue;
        };

        let time = (|| -> Result<_> {
            Ok(str::from_utf8(suffix)?
                .parse::<jiff::Timestamp>()?
                .to_zoned(jiff::tz::TimeZone::system()))
        })()
        .with_context(|| {
            format!("failed to parse time from name: {file_name:?} (prefix: {prefix:?})")
        })?;
        let file_name = file_name.to_str().expect("checked to be UTF-8");

        ensure!(
            open_dir(Some(target_dir_fd), file_name.as_ref())
                .and_then(ioctl::subvol_getflags)
                .is_ok(),
            "{file_name:?} is not a BTRFS subvolume",
        );

        if time.timestamp() > now {
            eprintln!("warning: ignore and keep {file_name:?} from the future");
            continue;
        }

        snaps.push(SnapshotInfo {
            file_name: file_name.to_owned(),
            time,
            keep_reason: String::new(),
        });
    }

    // Sort in reverse-time order.
    snaps.sort_unstable_by_key(|s| std::cmp::Reverse(s.time.timestamp()));

    Ok(snaps)
}

fn open_dir(dir: Option<BorrowedFd<'_>>, path: &Path) -> rustix::io::Result<OwnedFd> {
    use rustix::fs::{Mode, OFlags};

    rustix::fs::openat(
        dir.unwrap_or(rustix::fs::CWD),
        path,
        OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
}
