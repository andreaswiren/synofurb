#!/usr/bin/env python3
"""Drives `synofurb-cli wizard --demo` through a real pseudo-terminal, expect-style, with TERM=dumb and no colour.

  wizard_pty.py BINARY refuse     wrong confirmation phrase three times -> must exit 3, nothing rendered
  wizard_pty.py BINARY rehearse   correct answers -> must exit 0 and print the rehearsal (dry run) output
  wizard_pty.py BINARY menu       choose "Quit" at the goal menu -> must exit 0 and say nothing was changed

Prompts are recognised by the text of the last, unterminated output line; every prompt without a rule gets an empty answer
(= the default), which is how a person who just presses Enter would drive it.
"""
import os
import pty
import re
import select
import subprocess
import sys
import tempfile
import time

ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def drive(binary, args, rules, env_extra=None, timeout=60):
    data_dir = tempfile.mkdtemp(prefix="sf-wizpty-")
    env = {"PATH": "/usr/bin:/bin", "TERM": "dumb", "NO_COLOR": "1", "HOME": data_dir, "XDG_DATA_HOME": data_dir + "/data", "LANG": "C"}
    env.update(env_extra or {})
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(binary, [binary] + args, env)
    transcript = ""
    last_len = -1
    quiet_since = time.time()
    deadline = time.time() + timeout
    answered = 0
    status = None
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.25)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                chunk = b""
            if not chunk:
                break
            transcript += chunk.decode("utf-8", "replace")
            quiet_since = time.time()
            continue
        # no output for a moment: is the child waiting at a prompt?
        done, st = os.waitpid(pid, os.WNOHANG)
        if done:
            status = st
            break
        if time.time() - quiet_since < 0.4 or len(transcript) == last_len:
            continue
        last_len = len(transcript)
        tail = ANSI.sub("", transcript).replace("\r", "").split("\n")[-1]
        if not tail.strip():
            continue
        answer = ""
        for pattern, ans in rules:
            if re.search(pattern, tail):
                answer = ans
                break
        os.write(fd, (answer + "\n").encode())
        answered += 1
        transcript += "<<" + answer + ">>\n"
    if status is None:
        try:
            _, status = os.waitpid(pid, 0)
        except ChildProcessError:
            status = 0
    code = os.waitstatus_to_exitcode(status) if status is not None else -1
    return code, ANSI.sub("", transcript).replace("\r", ""), answered


def fail(msg, transcript):
    print("FAIL:", msg)
    print("----- transcript (tail) -----")
    print("\n".join(transcript.splitlines()[-40:]))
    sys.exit(1)


def main():
    binary, mode = sys.argv[1], sys.argv[2]
    if mode == "menu":
        code, t, _ = drive(binary, ["wizard", "--demo"], [(r"choice \[1\]: $", "4")])
        if code != 0 or "Nothing was changed" not in t:
            fail(f"quit at the goal menu: exit {code}", t)
        print("ok  wizard menu: quit exits 0")
        return
    if mode == "refuse":
        rules = [
            (r"verified backup.*\[y/N\]: $", "y"),
            (r"Do you understand.*\[y/N\]: $", "y"),
            (r"Type ERASE DS918\+ exactly", "ERASE DS923+"),
        ]
        code, t, _ = drive(binary, ["wizard", "--demo"], rules)
        if code != 3:
            fail(f"wrong phrase must exit 3, got {code}", t)
        for must in ["Step 7 of 9", "CONFIRMATION", "serial 1920PDN123456", "that is not the phrase", "Nothing was changed"]:
            if must not in t:
                fail(f"missing {must!r}", t)
        if "Step 8 of 9" in t or "stage2.sh -----" in t:
            fail("the wizard went on after a wrong phrase", t)
        print("ok  wizard refuses a wrong phrase (exit 3), nothing rendered")
        # declining the backup question also refuses
        code, t, _ = drive(binary, ["wizard", "--demo"], [(r"verified backup.*\[y/N\]: $", "n")])
        if code != 3 or "Step 8 of 9" in t:
            fail(f"declining the backup question must exit 3, got {code}", t)
        print("ok  wizard refuses when the backup question is declined (exit 3)")
        return
    if mode == "rehearse":
        rules = [
            (r"verified backup.*\[y/N\]: $", "y"),
            (r"Do you understand.*\[y/N\]: $", "y"),
            (r"Type ERASE DS918\+ exactly", "ERASE DS918+"),
            (r"Host name of the new system", "vault"),
        ]
        code, t, _ = drive(binary, ["wizard", "--demo"], rules)
        if code != 0:
            fail(f"rehearsal must exit 0, got {code}", t)
        for must in ["Step 9 of 9", "Rehearsal finished", "----- stage2.sh -----", "DRY RUN"]:
            if must not in t:
                fail(f"missing {must!r}", t)
        if "\x1b" in t:
            fail("colour codes with TERM=dumb", t)
        print("ok  wizard rehearsal completes (exit 0) and renders the dry run")
        return
    print("unknown mode", mode)
    sys.exit(2)


if __name__ == "__main__":
    main()
