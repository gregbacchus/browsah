# Web apps

A web app is a workspace for one site. Its window shows the page and little else, it has its own icon and its
own entry in the desktop's launcher, and the desktop (GNOME) shows it as an application of its own, not as part
of the browser. Underneath it is an ordinary workspace (`workspace.yaml`, see [workspace.md](workspace.md)) with an
`app:` section, so everything a workspace can do applies: cookie rules, isolated storage, permissions, routing rules,
a window colour, a popup policy.

```bash
browsah app add mail --profile=work --url https://mail.example.com/ --name "Mail" \
    --scope "mail.example.com,accounts.example.com/signin/*" --display standalone \
    --color "#2a6f97 #61a5c2" --icon ./mail.png
browsah app list --profile=work
browsah app install mail --profile=work     # write the launcher entry and icon again
browsah app remove mail --profile=work --yes
```

`app add` creates the workspace, writes `~/.local/share/applications/be.geee.browsah.<profile>.<id>.desktop`
and the icon (the file given with `--icon`, or a generated one from the colour and the first letter of the name).
Start it from the launcher, or with `browsah --profile=<profile> --workspace=<id>`.

**Installing from a page.** Right-click a tab and choose *Install as web app…*. The page's web manifest is read
(through DevTools, as Chromium sees it): its name, start page, scope, display mode, theme colour and the best icon.
You are shown the name and start page and can change the name before anything is saved; a site without a
manifest can be installed too, as its own page. The result is exactly what `browsah app add` makes. The manifest
is the site's own claim, so it is read once, at install time: the saved configuration is yours to edit, and
the site cannot change the app later. Icons marked only `maskable` or `monochrome` are not used (GNOME does not mask).

What the window does:

- The start page is the one **pinned tab**, limited to the app's scope (by default the start page's site).
  A link that leaves the scope opens in a window of its own, or wherever a routing rule sends it.
- No new tabs; popups open as modals over the app (a payment or sign-in window); controls follow `app.display`.
- It is a window of its profile's process and shares the profile's cookies unless it sets `storage: isolated`.
- Its window announces its own application id, `be.geee.browsah.<profile>.<id>`, which is also the name of its
  `.desktop` file; that is how GNOME groups the window under the app's own icon.

The options are the `app.*` rows of [workspace.md](workspace.md#web-apps-app); an example is
[`examples/webapp.yaml`](../../examples/webapp.yaml). Web apps do not appear in a tab's *Send to workspace* list.
