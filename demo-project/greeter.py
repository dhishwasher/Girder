"""Demo module for the Author tab / bitcode do walkthrough.

This project exists so a real (non-dry) authored run has somewhere safe to
write: it is disposable and read by nothing under tools/ or docs/, unlike
sample-project/, which tools/authoring_task_check.py and
tools/plan_executor_oracle.py pin as a measurement baseline (see gap 18/21
in docs/core-gap-analysis.md).
"""


def format_greeting(name):
    return f"hello {name}"


def shout_greeting(name):
    return format_greeting(name).upper()


def farewell(name):
    return f"goodbye {name}"
