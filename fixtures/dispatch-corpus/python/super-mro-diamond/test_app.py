class Base:
    def greet(self):
        return "base"


class Left(Base):
    def greet(self):
        return "left-" + super().greet()


class Right(Base):
    def greet(self):
        return "right-" + super().greet()


class Diamond(Left, Right):
    pass


def test_diamond_mro():
    d = Diamond()
    assert d.greet() == "left-right-base"
