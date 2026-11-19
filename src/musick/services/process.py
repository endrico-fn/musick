"""Bounded subprocess helpers: timeouts must always reap the child process."""

from __future__ import annotations

import asyncio
import contextlib


async def communicate_capped(
    proc: asyncio.subprocess.Process, timeout: float
) -> tuple[bytes, bytes]:
    """Await `proc.communicate()`, killing and reaping the child on timeout.

    `asyncio.wait_for(proc.communicate(), ...)` only cancels the coroutine, so a
    hung child would keep running and holding sockets. This guarantees the
    process is killed and reaped before propagating ``TimeoutError``.
    """
    try:
        async with asyncio.timeout(timeout):
            return await proc.communicate()
    except TimeoutError:
        with contextlib.suppress(ProcessLookupError):
            proc.kill()
        with contextlib.suppress(OSError, RuntimeError):
            await proc.wait()
        raise
