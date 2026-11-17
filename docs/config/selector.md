# The profile selector

Browsah profiles are separate applications, so when another program opens a web link you need to say
which profile gets it. The **selector** is a small program, registered as the system's web browser,
that asks. It starts instantly (it does not load Chromium), lists your profiles, and starts the one
you pick with the link.

```bash
browsah selector install        # add the "Browsah" desktop entry and icon
browsah selector status         # is it installed, and which browser is the default?
browsah selector uninstall
```

After `install`, choose **Browsah** as the default web browser in your desktop's settings (Default
Apps), or run `xdg-settings set default-web-browser be.geee.browsah.desktop`. Changing the default
browser is left to you: `install` does not do it. (Each profile's own entry, `Browsah (Name)`, also
declares the web handlers, so you can choose a single profile as the default instead and skip the
question.)

`browsah select [URL]` is what the entry runs. In the window, press `1` to `9`, click a profile or
press `Enter` to choose, `Esc` to cancel. A profile that is running says so. The **default profile** is listed first and carries a *Default* badge: the profile named by
`default_profile` in `config.yaml` if it exists, else the profile called `default` (what a plain `browsah` starts). (When
`default_profile` is set the selector does not ask at all, see below, so in the list the badge is on `default`.)

It does not ask when it can already tell:

1. **One profile only:** it opens there.
2. **An application rule** (`routing:` in `config.yaml`) that names a profile for the link:
   `open: profile:work` or `open: profile:work/workspace:mail` sends it there; `open: block` drops it.
   Rules that open a workspace or ephemeral window do not decide here (they depend on a profile).
   See [routing.md](routing.md).
3. **`default_profile`** in `config.yaml` ([options](options.md#behaviour)): it opens there.

Otherwise the window appears. Once a profile has the link, that profile's own routing rules apply
(including opening sign-ins started by other applications in an ephemeral window).
