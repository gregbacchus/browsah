# Workspace picker focus on Wayland

**Date:** 2026-11-20

The picker lives in an overlay on the source window. A native GTK popover can give a mouse click
its own Wayland surface. GDK's default focus request uses a seat serial and the keyboard-focused
surface, and Mutter can refuse activation if that serial and surface do not match. Keeping the
picker on the toplevel makes the row click belong to the same surface as the window's keyboard
focus. GDK can then request ordinary compositor activation.

On the GNOME Wayland development machine, an initial protocol trace showed the scripted picker
test issuing requests with serial zero. Scripted GTK button signals cannot supply a physical input
serial. The targeted picker test also checks the selection and reuse of an existing window.

```bash
test_dir="$(mktemp -d)"
XDG_CONFIG_HOME="$test_dir/config" XDG_DATA_HOME="$test_dir/data" \
XDG_CACHE_HOME="$test_dir/cache" BROWSAH_SHOTS="$test_dir/shots" scripts/test selftest picker
```
