#!/bin/bash
export INCLUDE_DEBUG_SYMBOLS=1
export RELEASE_BUILD=0
cd "$(dirname "$0")"
make "$@"