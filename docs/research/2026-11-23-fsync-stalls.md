# "Not Responding" from disk waits on the GTK thread (2026-11-23)

**Question:** why does GNOME show "Not Responding" for browsah many times a day, often after a page refresh or a notification?

**Environment:** the user's Work machine (`meganova`, Ubuntu, GNOME Wayland, browsah 0.5.2, profiles `work` and `home`). Not the
machine browsah is developed on (`kaizen`, where the journal showed three whole-process freezes in two days and no disk waits).

## Measured

`journalctl --user -t browsah --since today | grep "main loop"` on `meganova`, 11:02 to 14:26: about 20 stalls in the `work` profile and 2
in `home`, and every one of them has the GTK main thread in **state D (disk wait), system call 74** (`fsync` on x86-64), waiting in
`folio_wait_bit_common` (writeback of the file's pages) or `jbd2_log_wait_commit` / `add_transaction_credits` (the ext4 journal commit).
The "has not run" line is written at 750 ms and the "running again" line when the loop resumes; they are up to 11 s apart (11:02:59 to
11:03:10, 11:57:16 to 11:57:22, 12:08:20 to 12:08:30), so the stalls were 5 to 12 s: longer than GNOME's 5 s ping timeout. Two profiles
stalled in the same second at 13:38:49 (one disk, shared journal).

## Cause (inferred from the above, not reproduced here)

The profile's SQLite database (`data.sqlite`, WAL mode) is written on the GTK thread: tabs (`save_now`, debounced), history on every page
load, usage, favicons, window state. SQLite's default `synchronous=FULL` makes each commit wait for the disk to confirm the write. When the disk's journal is
busy (Chromium's own caches write constantly, and a refresh makes a burst) one `fsync` waits seconds, the main loop cannot answer the
compositor's ping, and the dialog appears. A page refresh or a notification is when the burst of writes happens.

## Change

`Store::init` sets `PRAGMA synchronous = NORMAL` (checked by `store::tests::the_store_does_not_wait_for_the_disk_on_every_commit`). In WAL
mode that syncs only when the log is folded into the database file, not at each commit. A crash of browsah loses nothing; a power cut
can lose the last few changes (the tab list or a history entry) but cannot corrupt the file.

**Not verified:** that the stalls stop on `meganova` (this machine's disk never showed a wait, so there is nothing to compare here).
**Still possible:** the checkpoint that folds the log in (about every 4 MB of log) still syncs on the committing thread, so rare
stalls could remain; the fix for that would be to write on a worker thread. After the change, `journalctl --user -t browsah | grep
"main loop"` should show no `fsync` lines, and if some remain their wait channel says whether it is a checkpoint.

Reproduce: the journal command above; `scripts/ping-watch.py` shows the compositor's pings against our answers.
