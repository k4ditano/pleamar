# Language tests

One small `.plm` per thing the language promises. The first line says what is expected:

```
// expect: ok
// expect: error «the records of «rows» have no field»
// expect: note «there is already a `let wide` in»
```

`./run-tests.sh` runs them all through `pleamar --check` and says which ones do not hold. A note is something said on loading a scene that still loads. An expected error or note is checked by a piece of its message: that way we also watch that errors keep saying something useful.

When a language bug gets fixed, its minimal case ends up here.

The libraries the tests import live in `tests/common/`; those are not checked on their own, because a library is not opened: it is imported.
