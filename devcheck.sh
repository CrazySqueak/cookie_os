#!/bin/bash
export CARGOFLAGS="$CARGOFLAGS --color=always"
./makedev.sh check 2>&1 | less --raw-control-chars --quit-on-intr --redraw-on-quit
