"""Known clone and non-clone families."""


def normalize_primary(value: str) -> str:
    cleaned = value.strip()
    lowered = cleaned.lower()
    return lowered.replace("-", "_")


def normalize_secondary(value: str) -> str:
    cleaned = value.strip()
    lowered = cleaned.lower()
    return lowered.replace("-", "_")


def unrelated_total(values: list[int]) -> int:
    total = 0
    for value in values:
        total += value * 2
    return total
