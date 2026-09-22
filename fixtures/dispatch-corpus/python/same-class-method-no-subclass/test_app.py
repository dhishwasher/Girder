class Widget:
    def render(self):
        return "widget"


def test_render():
    w = Widget()
    assert w.render() == "widget"
