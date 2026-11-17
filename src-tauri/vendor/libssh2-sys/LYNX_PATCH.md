# LYNX local libssh2 transport patch

- Base crate: `libssh2-sys 0.3.3`
- Crate source commit recorded by the published archive: `1abdbdb031cfea7e9e3dca7980ea36845bba1da8`
- Upstream native source bundled by that crate: `libssh2 1.11.1`

`libssh2/src/transport.c` used `MIN(current, remaining)` for the integrated-MAC
receive path. A partial encrypted packet can make `remaining` negative, after
which the following `assert(numdecrypt >= 0)` aborts the complete Windows
process. LYNX clamps the result to zero so the nonblocking transport waits for
the remaining bytes.

Do not remove the local `[patch.crates-io]` entry until the same lower-bound
check exists in the exact upstream source bundled by the replacement crate and
the native Windows SSH runtime gate has passed.
