from funpypi import setup

install_requires = ["funpypi","funfile","requests","tqdm"]

setup(
    name="funget",
    install_requires=install_requires,
    entry_points={
        "console_scripts": [
            "funget = funget.script:funget",
        ]
    },
)