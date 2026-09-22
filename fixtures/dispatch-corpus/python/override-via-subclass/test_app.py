class Shape:
    def area(self):
        return 0


class Square(Shape):
    def area(self):
        return 4


def render(shape):
    return shape.area()


def test_square():
    assert render(Square()) == 4


def test_base_shape():
    assert render(Shape()) == 0
