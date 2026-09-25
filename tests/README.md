# Language tests

One small `.plm` per thing the language promises. The first line says what is expected:

```
// expect: ok
// expect: error «the records of «rows» have no field»
```

`./run-tests.sh` runs them all through `pleamar --check` and says which ones do not hold. An expected error is checked by a piece of its message: that way we also watch that errors keep saying something useful.

When a language bug gets fixed, its minimal case ends up here.

The libraries the tests import live in `tests/common/`; those are not checked on their own, because a library is not opened: it is imported.
