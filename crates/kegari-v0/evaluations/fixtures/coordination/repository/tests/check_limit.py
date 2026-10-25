from src.limit import within_limit
assert within_limit(3, 3)
assert within_limit(2, 3)
assert not within_limit(4, 3)
