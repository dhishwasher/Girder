import unittest

from greeter import farewell, format_greeting, shout_greeting


class GreeterTests(unittest.TestCase):
    def test_format_greeting(self):
        self.assertEqual(format_greeting("Ada"), "hello Ada")

    def test_shout_greeting_delegates_to_format_greeting(self):
        self.assertEqual(shout_greeting("Ada"), "HELLO ADA")

    def test_farewell(self):
        self.assertEqual(farewell("Ada"), "goodbye Ada")


if __name__ == "__main__":
    unittest.main()
