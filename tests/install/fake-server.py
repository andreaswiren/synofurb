#!/usr/bin/env python3
"""Tiny static HTTP server for the installer tests: fake-server.py DIR PORTFILE  (binds 127.0.0.1, random port)."""
import functools
import http.server
import sys


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, fmt, *args):  # keep test output readable
        pass


def main() -> None:
    directory, portfile = sys.argv[1], sys.argv[2]
    handler = functools.partial(Quiet, directory=directory)
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler) as srv:
        with open(portfile, "w", encoding="utf-8") as fh:
            fh.write(str(srv.server_address[1]))
        srv.serve_forever()


if __name__ == "__main__":
    main()
