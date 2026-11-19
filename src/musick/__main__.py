"""Package entrypoint: `python -m musick` or `musick`."""

from __future__ import annotations

from musick.app import MusickApp
from musick.ui.widgets import init_terminal_image_support


def main() -> None:
    init_terminal_image_support()
    MusickApp().run()


if __name__ == "__main__":
    main()
