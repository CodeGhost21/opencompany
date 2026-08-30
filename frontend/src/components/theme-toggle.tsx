import { Monitor, Moon, Sun } from "lucide-react";
import { useTheme } from "next-themes";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

/**
 * Light / dark / system switch, following the OS by default.
 *
 * The two icons are stacked and cross-faded by the `dark:` variant rather than
 * swapped in JS, so the glyph is correct on the first paint — `useTheme()` has
 * no resolved theme until after mount, and branching on it flashes the wrong
 * icon. That puts the Moon in `absolute`, which makes `relative` on the trigger
 * load-bearing: `Button` is `position: static`, so without it the Moon resolves
 * against whatever positioned ancestor happens to be up the tree. On Settings
 * that is `SidebarInset`, *outside* the view's own `overflow-y-auto` scroller —
 * so the glyph did not scroll with its button and drifted down the page by
 * exactly `scrollTop`, leaving an empty 32x32 hit target behind (issue #922).
 */
export function ThemeToggle() {
  const { setTheme } = useTheme();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant="ghost" size="icon" className="relative" aria-label="Change theme" />
        }
      >
        <Sun className="size-4 rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
        <Moon className="absolute size-4 rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => setTheme("light")}>
          <Sun className="size-4" /> Light
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("dark")}>
          <Moon className="size-4" /> Dark
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("system")}>
          <Monitor className="size-4" /> System
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
