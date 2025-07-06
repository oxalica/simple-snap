# simple-snap

Minimalist BTRFS periodic snapshot tool. Snapshot, prune, and nothing else.

This tool is for ones who know how BTRFS subvolumes and snapshots work.
The main purpose of this periodic snapshot tool is to:
- Save data from accidental data loss: `rm`, `git reset --hard`.
- Local rollback of inconsist program data: `git commit` breaks the repository
  during a power loss, game save bricks after a crash.

This tool does NOT:
- Save you from hardware errors: bit-flip in memory, bit-rot on faulty disk.
- Allow global rollback of the whole subvolume or even the root subvolume.
  If you don't know why not, then always do not. There be dragons!

## Usages

### Create snapshots

```console
$ simple-snap --target-dir /.snapshots --source /home/alice --prefix home-alice@

created snapshot "/.snapshots/home-alice@2025-07-06T00:29:43.190772445-04:00" for "/home/alice"
```

### Prune snapshots

Note that subvolume deletion requires either root permission, or mount option
`user_subvol_rm_allowed` is used and the effective user owns the subvolume.

The prune policy mimic options of `restic forget --keep-*`.
See [their documentation](https://restic.readthedocs.io/en/v0.18.0/060_forget.html#removing-snapshots-according-to-a-policy)
or run `simple-snap prune --help` for detail explanations.

```console
# simple-snap prune \
    --target-dir /.snapshots \
    --prefix home-alice@ \
    --keep-within 6h \
    --keep-daily 7 \
    --keep-hourly 48 \
    --keep-last 2

home-alice@2025-07-06T00:30:06.123456789-04:00 last-within,last-n,hourly,daily
home-alice@2025-07-06T00:15:06.123456789-04:00 last-within,last-n
home-alice@2025-07-06T00:00:03.123456789-04:00 last-within
home-alice@2025-07-05T23:45:16.123456789-04:00 last-within,hourly,daily
[..]
home-alice@2025-06-29T06:00:01.123456789-04:00 !!delete!!
---
would keep 69 of 74 snapshots, and delete 5 snapshots.
deleted 5 snapshots (no commit).
```

Note that we do not commit subvolume deletion. I personally think it unnecessary
and may hurt disk performance when running `prune` periodically.
You may run `btrfs filesystem sync /path/to/btrfs` yourself if you do want a
BTRFS transaction commit.

### Periodical execution

You are expected to use [cron](https://wiki.archlinux.org/title/Cron) or
[systemd timer](https://wiki.archlinux.org/title/Systemd/Timers) to run simple-snap periodically.

### Time zone awareness

All timestamps, date time options use local time zone with an (automatic)
explicit offset, so it is unambiguous when switching time zones and also easy to
understand for human, if you are not in the +00:00 zone.

If you are a UTC-fan, set environment variable `TZ=Etc/UTC` to force all date time to use UTC.

We also use specified units for calendar arithmetic. This means, for
example, `--keep-with-in 1mo` will keep snapshots from `(month-1)-(day)` to
`(month)-(day)`, no matter how many days there are in the last month.

## Minimalism

- The code is ~500LoC and should be easy to reviewable.

  Since `prune` requires root, please review it by yourself before use, especially the `unsafe` parts.

- We call ioctl directly, thus has no btrfs-progs dependency.

- `unsafe` only comes from 3 BTRFS ioctls, under `src/ioctl.rs`.
  All used ioctls are [officially documented](https://btrfs.readthedocs.io/en/latest/btrfs-ioctl.html).
