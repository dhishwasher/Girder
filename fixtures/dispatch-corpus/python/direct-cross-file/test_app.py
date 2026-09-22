from app import target


def test_cross_file():
    assert target() == 42
