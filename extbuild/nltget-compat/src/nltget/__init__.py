"""Compatibility namespace for the renamed funget package."""

import warnings

warnings.warn("nltget was renamed to funget", DeprecationWarning, stacklevel=2)

from funget import *  # noqa: E402,F401,F403
from funget import __all__, __path__  # noqa: E402,F401
