import click


class PyWidget:
    def render(self) -> str:
        return "python"


def py_helper() -> str:
    return "python"


def py_entry() -> str:
    return py_helper()


@click.command()
def py_ship() -> str:
    return py_helper()


def py_isolated() -> int:
    return 7
