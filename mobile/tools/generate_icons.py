#!/usr/bin/env python3
"""Compatibility entrypoint for the shared icon pipeline."""
from pathlib import Path
import runpy

runpy.run_path(str(Path(__file__).resolve().parents[2] / 'scripts/generate-icons.py'), run_name='__main__')
