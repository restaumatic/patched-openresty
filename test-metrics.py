#!/usr/bin/env python3
"""End-to-end check of ngx_metrics: runs nginx, sends requests and reads the metrics over LuaJIT FFI.

Usage: ./test-metrics.py [nginx-binary]   (default: the dev-make build)
"""

import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OPENRESTY = ROOT / "openresty-1.27.1.1"
LUAJIT_LIB = OPENRESTY / "build/luajit-root/usr/local/openresty/luajit/lib"

CONFIG = """
daemon off;
master_process off;
error_log logs/error.log info;
pid logs/nginx.pid;
events {}
http {
  access_log off;
  server {
    listen 127.0.0.1:%(port)d;
    location /metrics {
      content_by_lua_block {
        local ffi = require "ffi"
        ffi.cdef[[
          char *ngx_metrics_prometheus(int reset);
          void ngx_metrics_free_result(char *metrics);
        ]]
        local metrics = ffi.gc(ffi.C.ngx_metrics_prometheus(0), ffi.C.ngx_metrics_free_result)
        ngx.print(ffi.string(metrics))
      }
    }
  }
}
"""


def run(nginx, workdir):
    """Starts nginx, sends a few requests and returns ({metric+tags: (count, sum)}, stderr, error log)."""
    port = random.randint(20000, 40000)
    logs = workdir / "logs"
    shutil.rmtree(logs, ignore_errors=True)
    logs.mkdir()
    (workdir / "nginx.conf").write_text(CONFIG % {"port": port})
    url = f"http://127.0.0.1:{port}/metrics"

    proc = subprocess.Popen(
        [nginx, "-p", workdir, "-c", "nginx.conf"],
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        for _ in range(50):
            try:
                urllib.request.urlopen(url, timeout=1)
                break
            except OSError:
                time.sleep(0.1)
        for _ in range(3):
            urllib.request.urlopen(url, timeout=1)
        body = urllib.request.urlopen(url, timeout=1).read().decode()
    finally:
        proc.terminate()
        _, stderr = proc.communicate(timeout=5)

    values = dict(line.rsplit(" ", 1) for line in body.splitlines())
    metrics = {}
    for key, value in values.items():
        name, sep, tags = key.partition("{")
        if name.endswith("_count"):
            base = name.removesuffix("_count")
            metrics[base + sep + tags] = (int(value), int(values[base + "_sum" + sep + tags]))
    return metrics, stderr, (logs / "error.log").read_text()


def main():
    nginx = Path(sys.argv[1] if len(sys.argv) > 1 else OPENRESTY / "build/nginx-1.27.1/objs/nginx").resolve()
    if LUAJIT_LIB.is_dir():
        os.environ["LD_LIBRARY_PATH"] = ":".join(filter(None, [str(LUAJIT_LIB), os.environ.get("LD_LIBRARY_PATH")]))
    if not Path(f"{nginx}.symbols").exists():
        subprocess.run([OPENRESTY / "generate-symbols", nginx], check=True)

    with tempfile.TemporaryDirectory() as tmp:
        workdir = Path(tmp)

        print("== with symbols")
        metrics, stderr, error_log = run(nginx, workdir)
        assert "ngx_metrics:" not in error_log, error_log
        assert re.search(r"^num symbols: [1-9]", stderr, re.M), stderr
        for name in ["any_event_handler_time_ns", "event_loop_latency_ns"]:
            assert metrics[name][0] > 0, f"{name} not counted"
        for kind in ["total_wall", "self_wall", "total_cpu", "self_cpu"]:
            name = f'event_handler_{kind}_time_ns{{handler="ngx_event_accept"}}'
            assert metrics.get(name, (0, 0))[0] > 0, f"{name} not counted"
        # Self time is total minus children's time, so it can never exceed total.
        for name, (_, self_sum) in metrics.items():
            if name.startswith("event_handler_self_"):
                total_name = name.replace("_self_", "_total_", 1)
                assert self_sum <= metrics[total_name][1], f"self > total for {name}"

        print("== without symbols")
        copy = workdir / "nginx"
        shutil.copy(nginx, copy)
        metrics, _, error_log = run(copy, workdir)
        assert re.search(r"ngx_metrics: Failed to read .*nginx\.symbols", error_log), error_log
        assert metrics["any_event_handler_time_ns"][0] > 0, "builtin metrics stopped working"
        assert not any("handler=" in name for name in metrics), "handler metrics without symbols"

    print("OK")


if __name__ == "__main__":
    main()
