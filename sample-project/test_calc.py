import unittest

from calc import greet


class GreetTests(unittest.TestCase):
    def test_returns_uppercase(self):
        self.assertEqual(greet("world"), "HELLO WORLD")


if __name__ == "__main__":
    unittest.main()
