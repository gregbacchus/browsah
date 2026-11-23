# Shortcut audit: Ctrl+Home switched tabs (2026-11-24)

**Problem.** In a text box, `Ctrl+Home` (start of text) switched to the first tab.

**Cause (found by reading the code, not by reproducing it).** `adw::TabView` has a `shortcuts` property that defaults to
all of libadwaita's tab shortcuts, and we never set it. Besides `Ctrl+Tab`, `Ctrl+Page Up/Down`, `Ctrl+Shift+Page Up/Down`
(move tab) and `Alt+1..9/0`, that includes `Ctrl+Home`/`Ctrl+End` (first/last tab) and `Ctrl+Shift+Home`/`Ctrl+Shift+End`
(move tab to the start/end). `Ctrl+Home/End` move the caret to the start/end of a text field and `Ctrl+Shift+Home/End`
select to it, in every browser and toolkit. The page never saw them: the key reaches the page only if `input::is_shell_shortcut`
is false and nothing consumed it first.

**Fix.** `tab_view.set_shortcuts(ALL & !(CONTROL_HOME | CONTROL_END | CONTROL_SHIFT_HOME | CONTROL_SHIFT_END))` in `app/mod.rs`.
The `tabs` self-test section checks the property. A scripted key press cannot show the real behaviour (synthetic input on the
page's controller never reaches the TabView's own shortcuts): **not checked with a real keyboard**.

**The rest of the shortcuts** (our `set_accels_for_action` list in `app/mod.rs` and `app.new-window`/`app.quit-profile`),
against Chrome/Firefox and common text-editing keys. No other conflict with text editing: every chord is Ctrl or Alt plus a
letter, digit or function key that browsers also use that way.

| Shortcut | Here | Elsewhere | Verdict |
|---|---|---|---|
| Ctrl+L, F6 | address bar | same | standard |
| Ctrl+R, F5 | reload | same | standard |
| Ctrl+T, Ctrl+W, Ctrl+Shift+T | new, close, reopen tab | same | standard |
| Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+Page Up/Down | next/previous tab | same | standard |
| Alt+1..9, Alt+0 | tab 1..9 / last | Chrome: Ctrl+1..9; Firefox on Linux: Alt+1..9 | matches Firefox/Linux |
| Alt+Left/Right | back/forward | same | standard |
| Ctrl+D, Ctrl+H | bookmark, history | same | standard (Ctrl+H is also "backspace" in some terminals/emacs keymaps; not in GTK text fields) |
| Ctrl+F, Ctrl+G / F3, Ctrl+Shift+G / Shift+F3 | find | same | standard |
| Ctrl+P | print | same | standard |
| Ctrl+Shift+P | save as PDF | Firefox: private window | differs, no clash with text editing |
| Ctrl+Shift+I, F12 | DevTools | same | standard |
| F11 | full screen | same | standard |
| Ctrl+plus/equal/minus/0 (and keypad) | zoom | same | standard |
| Shift+Esc | task manager | Chrome: same | standard |
| Ctrl+Shift+N | new window | Chrome/Firefox: private window; Ctrl+N is a plain new window | differs; Ctrl+N is unused here |
| Ctrl+Q | quit profile | same | standard |
| Ctrl+Alt+C | copy page address | none | no clash (Ctrl+Shift+C is DevTools in other browsers, unused here) |

Left alone on purpose: `Ctrl+Page Up/Down` also move the caret/scroll in some editors, but every browser takes them for tabs.
Not added: `Ctrl+N`, `Ctrl+Shift+R` (hard reload), `Ctrl+1..9` (Chrome's tab jump), `Alt+Home`.

## Shortcuts inherited from GTK and libadwaita

Read from the code and the installed GTK 4.22.4 / libadwaita 1.9.1; **not tried with a real keyboard**.

- **`adw::TabView` (fixed above).** The only inherited set that acts *before* the page: the user's `Ctrl+Home` switched tabs even
  though the page's key handler takes every key that is not in `input::is_shell_shortcut`, so its controller runs ahead of the
  page's. Still on, on purpose: `Ctrl+Tab`/`Ctrl+Shift+Tab`, `Ctrl+Page Up/Down`, `Ctrl+Shift+Page Up/Down` (move the tab, as in
  Chrome), `Alt+1..9`, `Alt+0`.
- **`gtk::Paned` (split tabs)** binds `F6`, `F8`, `Ctrl+Tab`, arrows and `Home`/`End` on its handle. It only sees keys the page
  does not take, and the page takes all but the shell shortcuts, so nothing reaches it from a page. Not a conflict.
- **`adw::TabBar`/`AdwTabBox`** has arrow, `Home`/`End` and `Ctrl+Shift+Left/Right` (reorder) bindings when a tab has the keyboard
  focus. Page keys do not go there. Not a conflict.
- **Mnemonics, `F10` and primary menu buttons:** none are used (`use_underline`, `set_primary` do not occur).
- **The GTK inspector keys, `Ctrl+Shift+I` and `Ctrl+Shift+D`.** `org.gtk.gtk4.Settings.Debug enable-inspector-keybinding` is `true`
  on this machine (the default). `Ctrl+Shift+I` is also our DevTools shortcut, so it may open GTK's inspector as well, and
  `Ctrl+Shift+D` (Chrome: bookmark all tabs) may open it alone. **Unverified**: check by pressing both in a window. If the
  inspector appears, it needs turning off for the process.
