// A sprite whose color reflects a pass/fail outcome (003-visual-render, Slice 2). The agent computes
// `passed` from data it gathered, then paints a check (green) or a cross (red) accordingly — the same
// bitmap, a different palette entry.

let passed = true;   // e.g. derived from a prior tool call

let pal = palette();
pal.set(".", "transparent");
pal.set("#", if passed { "#5FAF5F" } else { "#D75F5F" });   // pollen vs sting

let icon = sprite(8, 8, pal);
if passed {
    icon.paint([          // a check mark
        "........",
        ".......#",
        "......##",
        "#....##.",
        "##..##..",
        ".####...",
        "..##....",
        "........",
    ]);
} else {
    icon.paint([          // a cross
        "........",
        ".#....#.",
        "..#..#..",
        "...##...",
        "...##...",
        "..#..#..",
        ".#....#.",
        "........",
    ]);
}

render(icon);
