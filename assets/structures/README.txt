Structures drawn in the structure editor (game --structures), one folder to a kind of
landmark and one file to a structure. They are compiled into the game: rebuild it after
saving to see them in the world. docs/DEVELOPMENT.md says how the editor works, how a
file reads and how the world chooses between these and the generator's own.

The kit folder is not structures: it holds the house kit's rules, one file to a region,
saying how its huts, timber houses and stone houses are built. They are written by hand,
read the same way (rebuild to see a change, under Houses in the editor), and
worldgen's settlements::kit says how one reads.
