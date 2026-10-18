# Simple Linux profiles

A system is **one flavor** plus any number of **mix-ins**, like Funtoo's `epro` profiles:

* a **flavor** says what kind of system it is and inherits from another one (`workstation` → `desktop` → `minimal`);
* a **mix-in** is an optional bundle (`gaming`, `office`, `dev`, ...) of packages and the USE flags they need;
* the **`gfx-*` mix-ins** are the graphics card: only one of them at a time (they share the group `gfx`) and each sets `VIDEO_CARDS`.

The installer's Software page, the `slp` command and the build of the binhost and of the live ISO all read the same files, which are
compiled into them (the `slp-profiles` crate), so there is one place that says what each choice means.

## Layout

```
profiles/
  flavor/index          the flavors, in the order they are offered, one per line
  flavor/<name>/...
  mix-in/index          the mix-ins, in the order they are offered
  mix-in/<name>/...
```

Every file of a profile directory is optional. Lines starting with `#` and empty lines are ignored.

| file | meaning |
|---|---|
| `desc` | first line: the name shown to the user; the other lines: a one-sentence description |
| `parent` | profiles this one inherits from (`flavor/minimal`, `mix-in/audio`), one per line, applied first |
| `atoms` | packages to install; `~atom` marks one that only has a `~amd64` keyword |
| `use` | `package.use` lines |
| `licenses` | `package.license` lines |
| `services` | OpenRC services to switch on after the packages are installed, in order |
| `make-conf` | `KEY="value"` lines for `/etc/portage/make.conf` (the graphics mix-ins set `VIDEO_CARDS`) |
| `needs` | `guru` (the package comes from the GURU overlay), `desktop` (only makes sense with a desktop) |
| `group` | mix-ins with the same group exclude each other |
| `desktop` | flavors only: `yes` or `no`, whether the system gets a desktop |
| `defaults` | flavors only: the mix-ins ticked by default (inherited and added to) |
| `hidden` | flavors only: not offered by the installer (`live` is the ISO's own) |
| `rebuild` | the packages to rebuild when the flavor's USE flags change (used by `iso/slim-rootfs.sh`) |

## Rules

* Anything is data here, not code. Adding a mix-in is adding a directory and one line in `mix-in/index`.
* `slp-profiles/tests/` compares the installer's old built-in package table and the binhost spec with what these files produce, so
  editing a file that changes what an install does fails a test until the expected files are updated on purpose.
