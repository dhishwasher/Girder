class Duck:
    def quack(self):
        return "quack"


class Person:
    def quack(self):
        return "I'm quacking"


def make_it_quack(quacker):
    return quacker.quack()


def test_duck():
    assert make_it_quack(Duck()) == "quack"
