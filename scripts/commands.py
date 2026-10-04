import os
import selectors
import signal
import subprocess
import time


def run(args, *, check=True, timeout=30, output_limit=256 * 1024, input=None,
        text=True, env=None, cwd=None):
    payload = input.encode() if isinstance(input, str) else input
    buffers = [bytearray(), bytearray()]
    deadline = time.monotonic() + timeout
    with selectors.DefaultSelector() as selector:
        process = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   stdin=subprocess.PIPE if payload else subprocess.DEVNULL,
                                   start_new_session=True, env=env, cwd=cwd)
        try:
            for index, stream in enumerate([process.stdout, process.stderr]):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ, index)
            offset = 0
            if payload:
                os.set_blocking(process.stdin.fileno(), False)
                selector.register(process.stdin, selectors.EVENT_WRITE, 2)
            while selector.get_map() or process.poll() is None:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError(f'{args[0]}: command timed out after {timeout:g}s')
                for key, _ in selector.select(min(remaining, 0.1)):
                    if key.data == 2:
                        try:
                            offset += os.write(key.fd, payload[offset:offset + 8192])
                        except BrokenPipeError:
                            offset = len(payload)
                        if offset == len(payload):
                            selector.unregister(key.fileobj)
                            key.fileobj.close()
                        continue
                    chunk = os.read(key.fd, 8192)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                        continue
                    if sum(map(len, buffers)) + len(chunk) > output_limit:
                        raise RuntimeError(f'{args[0]}: command output exceeds {output_limit} bytes')
                    buffers[key.data].extend(chunk)
            result = subprocess.CompletedProcess(args, process.returncode,
                                                 bytes(buffers[0]), bytes(buffers[1]))
            if text:
                result.stdout = result.stdout.decode(errors='replace')
                result.stderr = result.stderr.decode(errors='replace')
            if check:
                result.check_returncode()
            return result
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            for stream in [process.stdout, process.stderr, process.stdin]:
                if stream is not None:
                    stream.close()
