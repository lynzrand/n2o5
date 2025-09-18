# N2O4

> Under construction. Expect hiccups and unfinished parts.

A build system as library.

N2O4 is an idea grew out of fiddling [n2](https://github.com/evmar/n2).
Less ninjutsu, more oxidizer.

## Design notes

- **Not** `ninja`-compatibility-first.
- Improved ergonomics as a library, not as an executable.
- Be efficient with common cases, allow callbacks instead of forcing process calls everywhere.

## CLI and `ninja`

The `n2o4` commandline executable, located in `cli/`,
is used for testing and stressing the library.
It contains a subset of `ninja` for testing with the library and tweaking on ergonomics.

You may use it either as `n2o4 ninja [ninja_args...]`
or create a symlink whose name starts with `ninja` and use it as a drop-in replacement.

---

N<sub>2</sub>O<sub>4</sub> is also called nitrogen tetroxide (NTO)
when using as a propellant,
so in theory it's also "into".
