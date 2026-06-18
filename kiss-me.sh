#!/bin/sh
# SPDX-License-Identifier: GLWTPL

# Compatibility launcher. The mod manager now lives in Python.
exec "$(dirname "$0")/kiss-me.py" "$@"
