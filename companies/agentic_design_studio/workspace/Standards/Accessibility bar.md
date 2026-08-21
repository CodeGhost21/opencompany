# Accessibility bar

Requirements, not polish. Everything here constrains decisions made early, which
is why meeting it now is cheap and retrofitting it is not.

| Check | The failure it prevents |
| --- | --- |
| Contrast at the actual weight and size, including disabled and placeholder states | Text that tests fine as a swatch and fails as a design |
| Nothing distinguished by colour alone | A meaning a substantial number of people cannot perceive |
| Target size and spacing on the smallest supported device | A design tested with a cursor and used with a thumb |
| The whole task completable by keyboard, with visible focus | If you cannot finish it, neither can a screen-reader user |
| A reduced-motion answer for anything that moves | Parallax and autoplay reliably harm people |

Constraints that must survive the next revision go on `design-decisions` with
the requirement that produced them — otherwise the next round undoes them as
arbitrary. Anything that genuinely cannot be met is a `risks` row, never a
silent omission. See [[Design principles]] and [[Critique method]].
