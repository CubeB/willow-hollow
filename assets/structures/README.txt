Structures drawn in the structure editor (game --structures), one folder to a kind of
landmark and one file to a structure. They are compiled into the game: rebuild it after
saving to see them in the world. docs/DEVELOPMENT.md says how the editor works, how a
file reads and how the world chooses between these and the generator's own.

The house folder holds homes copied from the house kit and edited: each says the region
and step it was copied for, and keeps where its door and beds are. The world builds one
now and then among the kit's homes of that region and step. The folders of the
settlement buildings (tavern, smithy, sawmill, granary, market-hall, guild-house,
well-head, gate) are drawn ahead of their generators; the world does not build them yet.

The component folder holds regional components (porches, awnings, woodpiles...) copied
from the kit's and edited: each says which component it is, and is drawn about the
anchor it goes on, out from the house to the east. Wherever that component goes it is
chosen now and then against the kit's own, and only where it fits.

The kit folder is not structures: it holds the house kit's rules, one file to a region,
saying how its huts, timber houses and stone houses are built. They are written by hand,
read the same way (the editor's Houses view reads them again as they are saved; the
world takes them up when the game is built again), and worldgen's settlements::kit says
how one reads.
