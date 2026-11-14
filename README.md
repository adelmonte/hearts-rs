# hearts-rs

Windows 95 style Hearts, written in Rust as a single static binary (about 3.4 MB). It is a
rewrite of [qt-hearts](https://github.com/adelmonte/qt-hearts) built the same way as
[solitaire-rs](https://github.com/adelmonte/solitaire-rs): a native Wayland client and nothing
else, with no X11 and no XWayland. It doesn't load any shared libraries either: no Qt, no KDE
Frameworks, no libwayland, no libxkbcommon, no libc.so. The Wayland protocol, keyboard layouts
(XKB) and SVG rendering are all pure Rust inside the binary.

## The game

- You against three computer players (West, North and East) at **Easy**, **Medium** or
  **Hard**, the qt-hearts opponents ported move for move. Hard counts cards, plays by seat
  position, dumps points on the leader when it's behind and defends against a moon shot.
- Pass left, right, across, then hold, and the two of clubs leads. Hearts can't be led until
  one has been played (or the queen of spades, if you like). Shoot the moon to give everyone
  else 26.
- **Rules** from qt-hearts: the game ends at 50, 75, 100 or 150; landing exactly on the end
  score resets you to 50; the moon shooter may take -26 when +26 to the others would lose
  them the game; Full Polish (99 points and taking 25 resets you to 98).
- **Score sheet** after every hand, statistics (games, wins, average and best score, moons
  shot), undo and redo, a hint, and a demo that plays your cards for you.
- **KDE card decks**: anything in `carddecks/*/index.desktop` under `$XDG_DATA_HOME` or
  `$XDG_DATA_DIRS`, so system decks and decks from "Get New Card Decks" both show up. Plain
  `.svg` and compressed `.svgz` both work, and so do decks that keep their cards in `<defs>`.
- **KPat background themes**: anything in `kpat/themes/*.desktop`.
- On first run it reads your deck, difficulty, speed, rules and statistics from qt-hearts'
  `~/.config/Hearts/Hearts.conf`, if there is one. After that it keeps its own settings in
  `~/.config/hearts-rsrc` and never writes to qt-hearts' file.

## Windows 95

- **Menus, dialogs, push buttons and status bar** drawn by hand in the Windows 95 style, in a
  bitmap Helvetica (the X11 one), which is about as close as a free font gets to MS Sans Serif.
- **Passing as Windows plays it.** Click three cards to raise them, then the Pass button. The
  cards you get back stand out of your hand until you click OK.
- The status bar says what's going on: whose trick it was, why a card can't be played, when
  hearts are broken and when someone shoots the moon.
- **Raining cards when you win the game**, Windows 95 style. Click to dismiss it; double-click
  to deal again.
- **Card size is yours.** The cards follow the window until you pick a size: Ctrl++ and
  Ctrl+- grow and shrink them in 15% steps, and so does Ctrl+mouse wheel. The size is saved.
  Ctrl+0 fits the cards to the window again.

## Details

- West and East hold their cards turned sideways; a card turns face up and straightens as it
  travels to the trick, and cards that change hands pass over the others, not under them.
- Clicking while cards are moving snaps them into place and takes the click. Clicking the
  table while a finished trick is on show takes it in straight away.
- Fast, Normal and Slow use qt-hearts' pauses between plays, tricks and hands.
- Menus and dialogs hold the game until they close.
- Startup and zooming are quick because the deck renders on all cores. While you resize the
  window the old background is stretched, then re-rendered in a background thread once you
  stop.
- Only the parts of the window that changed are redrawn and sent to the compositor, and it
  uses no CPU while idle. Fractional scaling (125%, 150%...) renders at full resolution
  instead of being blurred by the compositor.
- A built-in Windows 95 style deck and the Clean Green background are compiled in, so the
  binary works with nothing else installed.

## Keys

| Key | Action |
| --- | --- |
| Ctrl+N, F2 | New game |
| Ctrl+Z / Ctrl+Shift+Z, Ctrl+Y | Undo / redo |
| H | Hint |
| D | Demo (on/off) |
| Left, Right, Home, End | Choose a card |
| Space | Select or play the chosen card |
| Enter | Pass / OK |
| Ctrl+Shift+S | Score sheet |
| Ctrl+Shift+T | Statistics |
| Ctrl++ / Ctrl+- / Ctrl+0 | Bigger / smaller / fit cards |
| F10, Menu key, right-click | Menu (opponents, speed, rules, deck, background and more) |
| F11, Ctrl+Shift+F | Full screen |
| F1 | Rules and keys |
| Ctrl+Q | Quit |

`hearts-rs --easy`, `--medium` and `--hard` pick the opponents.

## Building

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release
# -> target/x86_64-unknown-linux-musl/release/hearts-rs
```

`.cargo/config.toml` targets static musl by default. `cargo test` plays a few hundred games
at every difficulty and rule set, checking every card is legal and the score adds up, and
checks that the table fits the window without anything overlapping.

It needs a compositor with xdg-shell (every desktop compositor has it). Title bars come from
the compositor (KDE Plasma, sway, Hyprland and most others draw them). GNOME doesn't offer
server-side title bars, so there the window has none; use the keyboard or the right-click menu.

## Not included

qt-hearts' sound effects and its tilted cards on the trick pile, network play, and saving or
loading games.

## License

GPL-3.0-or-later; see `COPYING`. The built-in Clean Green background comes from KPat (by
Parker Coates), and the UI font is Adobe Helvetica from the X11 bitmap fonts. `NOTICE` has
the details.
