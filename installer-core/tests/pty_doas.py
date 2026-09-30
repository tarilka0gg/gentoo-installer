# Runs INSIDE the target chroot (see real_target.rs). Usage: pty_doas.py USER PASSWORD
# Runs `doas -u root id -un` as USER on a real pseudo-terminal, types PASSWORD at the
# prompt, and prints what happened. No `su`: it starts a new session and drops the
# controlling tty that doas prompts on.
import os, pty, pwd, select, sys, time

user, password = sys.argv[1], sys.argv[2].encode() + b"\n"
pw = pwd.getpwnam(user)
pid, fd = pty.fork()
if pid == 0:
    os.initgroups(user, pw.pw_gid); os.setgid(pw.pw_gid); os.setuid(pw.pw_uid)
    os.environ.update(HOME=pw.pw_dir, USER=user, LOGNAME=user, PATH="/usr/bin:/bin:/usr/sbin:/sbin", LANG="C")
    os.chdir(pw.pw_dir)
    os.execvp("/bin/sh", ["sh", "-c", "doas -u root id -un"])

out, answered, deadline = b"", 0, time.time() + 40
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 1.0)
    if fd in ready:
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
        prompts = out.lower().count(b"password:")
        if prompts > answered:
            answered = prompts
            os.write(fd, password)
    else:
        try:
            if os.waitpid(pid, os.WNOHANG)[0]:
                break
        except ChildProcessError:
            break

lines = [l.strip() for l in out.decode(errors="replace").replace("\r", "").splitlines() if l.strip()]
print("PROMPTS=%d" % out.lower().count(b"password:"))
print("RAN_AS_ROOT=%s" % ("yes" if "root" in lines else "no"))
print("RAW=" + " | ".join(lines)[-300:])
