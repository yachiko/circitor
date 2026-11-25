#!/usr/bin/env python3
"""Plan tool for the build-plan skill. Standard library only.

Reads the spec (specs/*.md, plus the facts docs docs/02-github-api.md, docs/03-clients.md and
docs/04-platform.md) and the plan (plan/tasks/*.md). Checks the plan against the spec, renders
its generated files, and tells an orchestrator what can start. Run it from the project root (the
directory holding specs/ and docs/), or pass --root.

  extract [--json FILE]           requirement inventory: IDs, parts, flags, decisions, open findings
  show ID [ID ...]                one requirement: where it is, its text, references, parts, tasks
  check [--scope P,P]             validate plan/tasks against the spec; exit status 1 on errors
  hints                           spec cross-references between tasks that may run at the same time
  index                           every task's frontmatter and Outcome, as one table (for helper briefs)
  render [--accept-spec-changes]  write plan/graph.json and plan/coverage.md, refresh README blocks
  ready                           tasks that can start now, most urgent first
  status ID STATUS                set one task's status
  diff                            spec changes since the plan's snapshot, and the tasks they touch

A requirement with sub-bullets (or a table of named scenarios after it, each row starting with a
`name` in backticks) has parts. A task can cover a whole requirement (`TST-4`) or some of its
parts (`TST-4:state-file`); a requirement counts as covered when some task lists it whole, or
every one of its parts is listed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import date
from pathlib import Path

# the facts docs (GitHub API, clients, platform): their sections are snapshotted as well
SNAPSHOT_DOCS = ["docs/02-github-api.md", "docs/03-clients.md", "docs/04-platform.md", "docs/07-notify.md"]
STATUSES = ["todo", "in-progress", "review", "done", "blocked", "dropped"]
KINDS = ["infra", "code", "test", "integration", "manual"]
SIZE_WEIGHT = {"S": 1, "M": 2, "L": 3}
REQUIRED_KEYS = ["id", "title", "kind", "status", "depends_on", "requirements", "owns"]
LIST_KEYS = {"depends_on", "requirements", "owns", "shares"}
KNOWN_KEYS = set(REQUIRED_KEYS) | LIST_KEYS | {"milestone", "size"}
REQUIRED_SECTIONS = ["Outcome", "Read first", "Scope", "Out of scope", "Acceptance"]
MAX_WHOLE_REQS = 25
README_BLOCKS = ["status", "waves", "critical", "graph", "hotspots", "open"]

TASK_ID_RE = re.compile(r"^T\d{2,3}$")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
FENCE_RE = re.compile(r"^\s*(```|~~~)")
BULLET_RE = re.compile(r"^(\s*)[-*+]\s+")
BULLET_DEF_RE = re.compile(r"^(\s*)[-*+]\s+(~~)?\*\*([A-Z]{2,4}-\d+)\*\*")
BOLD_ID_RE = re.compile(r"(~~)?\*\*([A-Z]{2,4}-\d+)\*\*")
U_ROW_RE = re.compile(r"^\|\s*(~~)?(U\d+)(~~)?\s*\|")
DEC_ROW_RE = re.compile(r"^\|\s*(D\d+)\s*\|")
FINDING_RE = re.compile(r"^###\s+(\S+)\s+·\s+(\w+)\s+·\s+(.+?)\s*$")
TABLE_PART_RE = re.compile(r"^\|\s*`([a-z][a-z0-9-]*)`\s*\|")
MENTION_RE = re.compile(r"\b([A-Z]{2,4})-(\d+)(?:\s*(?:to|–|-)\s*\1-(\d+))?\b")
U_MENTION_RE = re.compile(r"\bU(\d+)(?:\s*[–-]\s*U(\d+))?\b")
D_MENTION_RE = re.compile(r"(?<!log )\bD(\d+)\b")
PATH_RE = re.compile(r"\b((?:specs|docs|plan|testdata)/[A-Za-z0-9_./-]+?\.(?:md|py|json|toml|graphql))(#[\w-]+)?")


def die(msg: str) -> None:
    print(f"plan.py: {msg}", file=sys.stderr)
    sys.exit(2)


def norm(text: str) -> str:
    return re.sub(r"\s+", " ", text).strip()


def digest(text: str) -> str:
    return hashlib.sha256(norm(text).encode("utf-8")).hexdigest()[:16]


def anchor(title: str) -> str:
    a = strip_md(title).strip().lower()
    a = re.sub(r"[^\w\- ]", "", a)
    return a.replace(" ", "-")


def strip_md(s: str) -> str:
    s = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", s)
    for mark in ("**", "__", "`", "~~"):
        s = s.replace(mark, "")
    return s


def indent_of(line: str) -> int:
    return len(line.expandtabs(4)) - len(line.expandtabs(4).lstrip(" "))


def fence_mask(lines: list[str]) -> list[bool]:
    mask, inside = [False] * len(lines), False
    for i, line in enumerate(lines):
        if FENCE_RE.match(line):
            mask[i] = True
            inside = not inside
        elif inside:
            mask[i] = True
    return mask


def split_cells(line: str) -> list[tuple[str, int, int]]:
    """Table cells as (text, start, end) offsets into line; pipes inside backticks don't split."""
    cells, start, in_code, i = [], None, False, 0
    while i < len(line):
        ch = line[i]
        if ch == "\\":
            i += 2
            continue
        if ch == "`":
            in_code = not in_code
        elif ch == "|" and not in_code:
            if start is not None:
                cells.append((line[start:i].strip(), start, i))
            start = i + 1
        i += 1
    if start is not None and line[start:].strip():
        cells.append((line[start:].strip(), start, len(line)))
    return cells


def part_key(first_line: str) -> str:
    s = strip_md(BULLET_RE.sub("", first_line, count=1)).strip()
    m = re.search(r":|\s\(|,|;|\.(?:\s|$)", s)
    words = (s[: m.start()] if m else s).split()
    if not words or len(words) > 8:
        words = s.split()[:5]
    slug = re.sub(r"[^a-z0-9]+", "-", " ".join(words).lower()).strip("-")
    return slug[:48].rstrip("-") or "part"


# --- the spec ---------------------------------------------------------------------------------


class Spec:
    """Requirement inventory of specs/*.md, with decisions and open review findings."""

    def __init__(self, root: Path):
        self.root = root
        self.reqs: dict[str, dict] = {}
        self.sections: dict[str, dict] = {}
        self.decisions: list[dict] = []
        self.findings: list[dict] = []
        self.dangling: list[dict] = []
        self.duplicates: list[str] = []
        parsed = []
        for path in sorted((root / "specs").glob("*.md")):
            rel = path.relative_to(root).as_posix()
            lines = path.read_text(encoding="utf-8").split("\n")
            fence = fence_mask(lines)
            parsed.append((rel, lines, fence))
            self._sections(rel, lines, fence)
        for rel in SNAPSHOT_DOCS:
            if (root / rel).exists():
                lines = (root / rel).read_text(encoding="utf-8").split("\n")
                self._sections(rel, lines, fence_mask(lines))
        for rel, lines, fence in parsed:
            self._definitions(rel, lines, fence)
        self.prefixes = {r["prefix"] for r in self.reqs.values() if r["prefix"] != "U"}
        for r in self.reqs.values():
            r["refs"], r["decisions"] = self.mentions(r["text"], exclude=r["id"])
            for part in r["parts"]:
                part["refs"], _ = self.mentions(part["text"], exclude=r["id"])
        for rel, lines, _ in parsed:
            self._dangling(rel, lines)
        if (root / "specs" / "decisions.md").exists():
            self._decisions((root / "specs" / "decisions.md").read_text(encoding="utf-8").split("\n"))
        for path in sorted((root / "specs" / "reviews").glob("*.md")):
            self._findings(path)

    # sections: every heading's own text, so content outside requirement IDs is snapshotted too
    def _sections(self, rel: str, lines: list[str], fence: list[bool]) -> None:
        heads = [i for i, line in enumerate(lines) if not fence[i] and HEADING_RE.match(line)]
        bounds = ([0] if not heads or heads[0] != 0 else []) + heads
        seen: dict[str, int] = {}
        for n, start in enumerate(bounds):
            end = bounds[n + 1] if n + 1 < len(bounds) else len(lines)
            m = HEADING_RE.match(lines[start]) if start in heads else None
            body = "\n".join(lines[start:end])
            if not m and not body.strip():
                continue
            title = m.group(2) if m else ""
            a = anchor(title) if m else ""
            if a in seen:
                seen[a] += 1
                a = f"{a}-{seen[a]}"
            else:
                seen[a] = 0
            self.sections[f"{rel}#{a}"] = {
                "file": rel, "title": strip_md(title), "line": start + 1, "hash": digest(body),
            }

    def _definitions(self, rel: str, lines: list[str], fence: list[bool]) -> None:
        defs = []
        for i, line in enumerate(lines):
            if fence[i]:
                continue
            stripped = line.strip()
            if stripped.startswith("|"):
                m = U_ROW_RE.match(stripped)
                if m:
                    cells = split_cells(stripped)
                    defs.append({"id": m.group(2), "line": i, "form": "row",
                                 "text": " | ".join(c for c, _, _ in cells[1:]),
                                 "dropped": bool(m.group(1))})
                for m in BOLD_ID_RE.finditer(stripped):
                    cell_end = next((e for _, s, e in split_cells(stripped) if s <= m.start() < e),
                                    len(stripped))
                    defs.append({"id": m.group(2), "line": i, "form": "cell",
                                 "text": stripped[m.start():cell_end], "dropped": bool(m.group(1))})
                continue
            m = BULLET_DEF_RE.match(line)
            if m:
                defs.append({"id": m.group(3), "line": i, "form": "bullet",
                             "indent": indent_of(m.group(1)), "dropped": bool(m.group(2))})
                continue
            m = BOLD_ID_RE.search(line)
            if m and not BULLET_RE.match(line):
                defs.append({"id": m.group(2), "line": i, "form": "para", "dropped": bool(m.group(1))})

        def_lines = {d["line"] for d in defs}
        head_lines = {i for i, line in enumerate(lines) if not fence[i] and HEADING_RE.match(line)}
        section_of, current, seen = [], f"{rel}#", {}
        for i, line in enumerate(lines):
            if i in head_lines:
                a = anchor(HEADING_RE.match(line).group(2))
                if a in seen:
                    seen[a] += 1
                    a = f"{a}-{seen[a]}"
                else:
                    seen[a] = 0
                current = f"{rel}#{a}"
            section_of.append(current)

        for d in defs:
            parts: list[dict] = []
            if d["form"] == "bullet":
                end = self._bullet_end(lines, fence, d["line"], d["indent"], def_lines, head_lines)
                parts = self._child_parts(lines, fence, d["line"], end, d["indent"])
                trailing = self._trailing_parts(lines, fence, end + 1, def_lines, head_lines)
                text = "\n".join(lines[d["line"]:end + 1] + [p["text"] for p in trailing])
                parts += trailing
            elif d["form"] == "para":
                end = d["line"]
                while end + 1 < len(lines) and lines[end + 1].strip() and end + 1 not in def_lines \
                        and end + 1 not in head_lines:
                    end += 1
                text = "\n".join(lines[d["line"]:end + 1])
            else:
                text = d["text"]
            keys: dict[str, int] = {}
            for p in parts:
                k = p["key"]
                keys[k] = keys.get(k, 0) + 1
                if keys[k] > 1:
                    p["key"] = f"{k}-{keys[k]}"
                p["hash"] = digest(p["text"])
            rid = d["id"]
            if rid in self.reqs:
                self.duplicates.append(f"{rid}: {self.reqs[rid]['file']}:{self.reqs[rid]['line']} "
                                       f"and {rel}:{d['line'] + 1}")
                continue
            heading = self.sections.get(section_of[d["line"]], {}).get("title", "")
            flags_text = text + " " + heading
            self.reqs[rid] = {
                "id": rid,
                "prefix": "U" if rid.startswith("U") else rid.split("-")[0],
                "file": rel,
                "line": d["line"] + 1,
                "section": section_of[d["line"]],
                "form": d["form"],
                "dropped": d["dropped"],
                "proposal": "(proposal" in flags_text,
                "decision_needed": "(decision needed)" in text,
                "text": norm(text),
                "hash": digest(text),
                "parts": [{"key": p["key"], "hash": p["hash"], "text": norm(p["text"])} for p in parts],
            }

    @staticmethod
    def _bullet_end(lines, fence, i, indent, def_lines, head_lines) -> int:
        end, j = i, i + 1
        while j < len(lines) and j not in def_lines and j not in head_lines:
            line = lines[j]
            if not fence[j] and line.strip() and indent_of(line) <= indent:
                if BULLET_RE.match(line) or not line.lstrip().startswith("|"):
                    break
            if line.strip():
                end = j
            j += 1
        return end

    @staticmethod
    def _child_parts(lines, fence, start, end, indent) -> list[dict]:
        child = None
        for j in range(start + 1, end + 1):
            m = None if fence[j] else BULLET_RE.match(lines[j])
            if m and indent_of(m.group(1)) > indent:
                ci = indent_of(m.group(1))
                child = ci if child is None else min(child, ci)
        if child is None:
            return []
        parts, cur = [], None
        for j in range(start + 1, end + 1):
            line = lines[j]
            m = None if fence[j] else BULLET_RE.match(line)
            if m and indent_of(m.group(1)) == child:
                cur = {"key": part_key(line), "lines": [line]}
                parts.append(cur)
            elif cur is not None:
                if line.strip() and not fence[j] and indent_of(line) < child:
                    cur = None
                else:
                    cur["lines"].append(line)
        return [{"key": p["key"], "text": "\n".join(p["lines"])} for p in parts]

    @staticmethod
    def _trailing_parts(lines, fence, j, def_lines, head_lines) -> list[dict]:
        parts = []
        while j < len(lines) and j not in def_lines and j not in head_lines:
            m = None if fence[j] else TABLE_PART_RE.match(lines[j].strip())
            if m:
                parts.append({"key": m.group(1), "text": lines[j]})
            j += 1
        return parts

    def mentions(self, text: str, exclude: str = "") -> tuple[list[str], list[str]]:
        refs: set[str] = set()
        for m in MENTION_RE.finditer(text):
            if m.group(1) not in self.prefixes:
                continue
            lo = int(m.group(2))
            hi = int(m.group(3)) if m.group(3) and int(m.group(3)) > lo else lo
            refs.update(f"{m.group(1)}-{n}" for n in range(lo, hi + 1))
        for m in U_MENTION_RE.finditer(text):
            lo = int(m.group(1))
            hi = int(m.group(2)) if m.group(2) and int(m.group(2)) > lo else lo
            refs.update(f"U{n}" for n in range(lo, hi + 1) if f"U{n}" in self.reqs)
        refs.discard(exclude)
        decs = sorted({f"D{m.group(1)}" for m in D_MENTION_RE.finditer(text)}, key=id_sort)
        return sorted(refs, key=id_sort), decs

    def _dangling(self, rel: str, lines: list[str]) -> None:
        for i, line in enumerate(lines):
            for m in MENTION_RE.finditer(line):
                if m.group(1) in self.prefixes:
                    rid = f"{m.group(1)}-{m.group(2)}"
                    if rid not in self.reqs:
                        self.dangling.append({"file": rel, "line": i + 1, "id": rid})

    def _decisions(self, lines: list[str]) -> None:
        state = None
        for line in lines:
            m = HEADING_RE.match(line)
            if m:
                t = m.group(2).strip().lower()
                state = "resolved" if t.startswith("resolved") else "open" if t.startswith("open") else None
                continue
            m = DEC_ROW_RE.match(line.strip())
            if m and state:
                cells = [c for c, _, _ in split_cells(line.strip())]
                self.decisions.append({
                    "id": m.group(1), "state": state,
                    "title": norm(strip_md(cells[1])) if len(cells) > 1 else "",
                    "detail": norm(strip_md(cells[2])) if len(cells) > 2 else "",
                })

    def _findings(self, path: Path) -> None:
        log = path.stem
        cur = None
        for line in path.read_text(encoding="utf-8").split("\n"):
            m = FINDING_RE.match(line)
            if m:
                cur = {"log": log, "id": m.group(1), "severity": m.group(2), "title": m.group(3),
                       "location": "", "status": ""}
                self.findings.append(cur)
                continue
            if line.startswith("## ") or line.startswith("# "):
                cur = None
            elif cur is not None:
                if line.startswith("- Location:"):
                    cur["location"] = line.split(":", 1)[1].strip()
                elif line.startswith("- Status:"):
                    cur["status"] = line.split(":", 1)[1].strip()
        for f in self.findings:
            f["ids"], _ = self.mentions(f["location"] + " " + f["title"])

    def open_findings(self) -> list[dict]:
        return [f for f in self.findings if f["status"].startswith("open")]

    def parts_of(self, rid: str) -> list[str]:
        return [p["key"] for p in self.reqs[rid]["parts"]] if rid in self.reqs else []

    def snapshot(self) -> dict:
        return {
            "date": date.today().isoformat(),
            "requirements": {r["id"]: {"hash": r["hash"], "dropped": r["dropped"]} for r in self.reqs.values()},
            "parts": {f"{r['id']}:{p['key']}": p["hash"] for r in self.reqs.values() for p in r["parts"]},
            "sections": {k: s["hash"] for k, s in self.sections.items()},
            "decisions": {d["id"]: d["state"] for d in self.decisions},
            "findings": {f"{f['log']}:{f['id']}": f["status"] for f in self.findings},
        }


def id_sort(rid: str):
    m = re.match(r"^([A-Z]+)-?(\d+)", rid)
    return (m.group(1), int(m.group(2))) if m else (rid, 0)


# --- the plan ---------------------------------------------------------------------------------


class Problems:
    def __init__(self):
        self.items: list[tuple[str, str, str]] = []

    def add(self, level: str, where: str, msg: str) -> None:
        self.items.append((level, where, msg))

    def count(self, level: str) -> int:
        return sum(1 for lv, _, _ in self.items if lv == level)

    def show(self, levels=("ERROR", "WARN", "NOTE")) -> None:
        for level in levels:
            for lv, where, msg in self.items:
                if lv == level:
                    print(f"{lv:5} {where + ': ' if where else ''}{msg}")


def unquote(s: str) -> str:
    s = s.strip()
    if len(s) >= 2 and s[0] == s[-1] and s[0] in "\"'":
        return s[1:-1]
    return s


def strip_comment(line: str) -> str:
    if line.lstrip().startswith("#"):
        return ""
    m = re.match(r"^([^:\"']*:\s*)([\"'])(.*?)\2(.*)$", line)
    if m:  # a quoted value: only what follows the closing quote can be a comment
        rest = m.group(4)
        return m.group(1) + m.group(2) + m.group(3) + m.group(2) + re.sub(r"\s*#.*$", "", rest)
    return re.sub(r"\s#.*$", "", line)


def flow_list(val: str, n: int) -> list[str]:
    val = val.strip()
    if not (val.startswith("[") and val.endswith("]")):
        raise ValueError(f"line {n}: a list must look like [a, b]")
    inner = val[1:-1].strip()
    return [unquote(x) for x in inner.split(",") if x.strip()] if inner else []


def parse_frontmatter(text: str) -> tuple[dict, str, int]:
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        raise ValueError("no frontmatter: the file must start with a --- line")
    end = next((i for i in range(1, len(lines)) if lines[i].strip() == "---"), None)
    if end is None:
        raise ValueError("the frontmatter isn't closed with a --- line")
    data: dict = {}
    key, pending = None, None
    for n, raw in enumerate(lines[1:end], start=2):
        line = strip_comment(raw).rstrip()
        if not line.strip():
            continue
        if pending is not None:
            pending += " " + line.strip()
            if "]" in line:
                data[key] = flow_list(pending, n)
                pending = None
            continue
        m = re.match(r"^([A-Za-z_][\w-]*):(?:\s+(.*))?$", line)
        if m and not raw[:1].isspace():
            key, val = m.group(1), (m.group(2) or "").strip()
            if key in data:
                raise ValueError(f"line {n}: {key} is given twice")
            if not val:
                data[key] = []
            elif val.startswith("["):
                if "]" in val:
                    data[key] = flow_list(val, n)
                else:
                    pending = val
            else:
                data[key] = unquote(val)
            continue
        m = re.match(r"^\s+-\s+(.*)$", line)
        if m and key is not None and isinstance(data.get(key), list):
            data[key].append(unquote(m.group(1)))
            continue
        raise ValueError(f"line {n}: can't parse {raw.strip()!r}")
    if pending is not None:
        raise ValueError(f"{key}: the list isn't closed with ]")
    return data, "\n".join(lines[end + 1:]), end + 1


def clean_path(p: str) -> str:
    p = p.strip()
    while p.startswith("./"):
        p = p[2:]
    return p


def overlap(a: str, b: str) -> bool:
    an, bn = a.rstrip("/"), b.rstrip("/")
    if an == bn:
        return True
    if a.endswith("/") and bn.startswith(an + "/"):
        return True
    return b.endswith("/") and an.startswith(bn + "/")


def body_sections(body: str) -> dict[str, str]:
    out, cur, buf = {}, None, []
    for line in body.split("\n"):
        m = re.match(r"^##\s+(.+?)\s*$", line)
        if m:
            if cur is not None:
                out[cur] = "\n".join(buf)
            cur, buf = m.group(1).strip(), []
        elif cur is not None:
            buf.append(line)
    if cur is not None:
        out[cur] = "\n".join(buf)
    return out


class Plan:
    def __init__(self, root: Path, problems: Problems):
        self.root = root
        self.dir = root / "plan"
        self.tasks: dict[str, dict] = {}
        self.p = problems
        task_dir = self.dir / "tasks"
        for path in sorted(task_dir.glob("*.md")) if task_dir.is_dir() else []:
            self._load(path)

    def _load(self, path: Path) -> None:
        rel = path.relative_to(self.root).as_posix()
        try:
            data, body, _ = parse_frontmatter(path.read_text(encoding="utf-8"))
        except ValueError as e:
            self.p.add("ERROR", rel, str(e))
            return
        missing = [k for k in REQUIRED_KEYS if k not in data]
        if missing:
            self.p.add("ERROR", rel, f"frontmatter is missing {', '.join(missing)}")
            return
        for k in data:
            if k not in KNOWN_KEYS:
                self.p.add("WARN", rel, f"unknown frontmatter key {k!r}")
        for k in LIST_KEYS:
            v = data.get(k, [])
            data[k] = [v] if isinstance(v, str) and v else (v or [])
        tid = data["id"]
        if not TASK_ID_RE.match(tid):
            self.p.add("ERROR", rel, f"id {tid!r} isn't T followed by 2 or 3 digits")
        if not path.name.startswith(f"{tid}-"):
            self.p.add("ERROR", rel, f"the file name should start with {tid}-")
        if tid in self.tasks:
            self.p.add("ERROR", rel, f"id {tid} is also used by {self.tasks[tid]['file']}")
            return
        data.update({
            "file": rel,
            "body": body,
            "sections": body_sections(body),
            "owns": [clean_path(x) for x in data["owns"]],
            "shares": [clean_path(x) for x in data["shares"]],
            "milestone": data.get("milestone", ""),
            "size": data.get("size", ""),
        })
        self.tasks[tid] = data

    def live(self) -> dict[str, dict]:
        return {k: t for k, t in self.tasks.items() if t["status"] != "dropped"}


class Graph:
    """The dependency graph of the live (not dropped) tasks."""

    def __init__(self, plan: Plan, p: Problems):
        self.tasks = plan.live()
        self.deps = {t: [d for d in self.tasks[t]["depends_on"] if d in self.tasks] for t in self.tasks}
        self.order, self.cycle = self._topo()
        self.ok = self.cycle is None
        if not self.ok:
            p.add("ERROR", "", "dependency cycle: " + " → ".join(self.cycle))
            return
        self.anc: dict[str, set[str]] = {}
        for t in self.order:
            s: set[str] = set()
            for d in self.deps[t]:
                s.add(d)
                s |= self.anc[d]
            self.anc[t] = s
        self.level = {}
        for t in self.order:
            self.level[t] = 1 + max((self.level[d] for d in self.deps[t]), default=0)
        self.waves = [sorted([t for t in self.order if self.level[t] == n], key=id_sort)
                      for n in range(1, max(self.level.values(), default=0) + 1)]
        self.dependents = {t: sorted([u for u in self.tasks if t in self.deps[u]], key=id_sort)
                           for t in self.tasks}

    def _topo(self):
        state, order, stack = {}, [], []

        def visit(t):
            state[t] = 1
            stack.append(t)
            for d in self.deps[t]:
                if state.get(d) == 1:
                    return stack[stack.index(d):] + [d]
                if d not in state:
                    c = visit(d)
                    if c:
                        return c
            state[t] = 2
            stack.pop()
            order.append(t)
            return None

        for t in sorted(self.tasks, key=id_sort):
            if t not in state:
                c = visit(t)
                if c:
                    return [], c
        return order, None

    def ordered(self, a: str, b: str) -> bool:
        return a in self.anc[b] or b in self.anc[a]

    def weight(self, t: str) -> int:
        return SIZE_WEIGHT.get(self.tasks[t]["size"], 2)

    def critical_path(self) -> tuple[list[str], int]:
        best, prev = {}, {}
        for t in self.order:
            b, pv = 0, None
            for d in self.deps[t]:
                if best[d] > b:
                    b, pv = best[d], d
            best[t], prev[t] = b + self.weight(t), pv
        if not best:
            return [], 0
        end = max(sorted(best, key=id_sort), key=lambda t: best[t])
        chain = []
        while end:
            chain.append(end)
            end = prev[end]
        return chain[::-1], max(best.values())

    def remaining(self) -> dict[str, int]:
        """Longest weighted path from each task to the end, over tasks not done yet."""
        rem = {}
        for t in reversed(self.order):
            w = 0 if self.tasks[t]["status"] == "done" else self.weight(t)
            rem[t] = w + max((rem[u] for u in self.dependents[t]), default=0)
        return rem

    def reduced_edges(self) -> list[tuple[str, str]]:
        edges = []
        for t in self.order:
            for d in self.deps[t]:
                if not any(d in self.anc[e] for e in self.deps[t] if e != d):
                    edges.append((d, t))
        return edges


# --- check --------------------------------------------------------------------------------------


def coverage(spec: Spec, plan: Plan, p: Problems, scope: set[str] | None):
    whole: dict[str, list[str]] = {}
    parts: dict[str, list[str]] = {}
    for tid, t in sorted(plan.live().items(), key=lambda kv: id_sort(kv[0])):
        for ref in t["requirements"]:
            rid, _, key = ref.partition(":")
            if rid not in spec.reqs:
                p.add("ERROR", tid, f"{rid} isn't defined in specs/")
                continue
            if key:
                keys = spec.parts_of(rid)
                if key not in keys:
                    hint = f"; its parts are: {', '.join(keys)}" if keys else f"; {rid} has no parts"
                    p.add("ERROR", tid, f"{ref}: {rid} has no part {key!r}{hint}")
                    continue
                parts.setdefault(ref, []).append(tid)
            else:
                whole.setdefault(rid, []).append(tid)
            if spec.reqs[rid]["dropped"]:
                p.add("WARN", tid, f"{rid} is dropped (struck through) in {spec.reqs[rid]['file']}")
    uncovered, partial = [], []
    in_scope = [r for r in spec.reqs.values()
                if not r["dropped"] and (scope is None or r["prefix"] in scope)]
    for r in in_scope:
        rid = r["id"]
        if rid in whole:
            continue
        keys = spec.parts_of(rid)
        done = [k for k in keys if f"{rid}:{k}" in parts]
        if keys and len(done) == len(keys):
            continue
        if done:
            missing = [k for k in keys if k not in done]
            partial.append(rid)
            p.add("ERROR", "", f"{rid} is only partly covered; no task lists: "
                  + ", ".join(f"{rid}:{k}" for k in missing))
        else:
            uncovered.append(rid)
    if uncovered:
        p.add("ERROR", "", f"{len(uncovered)} requirements have no task: {', '.join(uncovered)}")
    shared = {rid: ts for rid, ts in whole.items() if len(ts) > 1}
    if shared:
        p.add("NOTE", "", "covered whole by several tasks (each task's Scope must say its share): "
              + "; ".join(f"{rid} {'+'.join(ts)}" for rid, ts in sorted(shared.items(), key=lambda kv: id_sort(kv[0]))))
    return {"in_scope": len(in_scope), "uncovered": uncovered, "partial": partial,
            "whole": whole, "parts": parts}


def check_tasks(spec: Spec, plan: Plan, g: Graph, p: Problems) -> None:
    tasks = plan.tasks
    unwritten = []
    for tid, t in sorted(tasks.items(), key=lambda kv: id_sort(kv[0])):
        if t["status"] not in STATUSES:
            p.add("ERROR", tid, f"status {t['status']!r} isn't one of {', '.join(STATUSES)}")
        if t["kind"] not in KINDS:
            p.add("ERROR", tid, f"kind {t['kind']!r} isn't one of {', '.join(KINDS)}")
        if t["status"] == "dropped":
            continue
        if not t["size"]:
            p.add("WARN", tid, "no size (S, M or L)")
        elif t["size"] not in SIZE_WEIGHT:
            p.add("ERROR", tid, f"size {t['size']!r} isn't S, M or L; split anything bigger than L")
        for d in t["depends_on"]:
            if d == tid:
                p.add("ERROR", tid, "depends on itself")
            elif d not in tasks:
                p.add("ERROR", tid, f"depends on {d}, which doesn't exist")
            elif tasks[d]["status"] == "dropped":
                p.add("ERROR", tid, f"depends on {d}, which is dropped; point at what replaced it")
            else:
                if tasks[d]["kind"] == "manual" and t["kind"] != "manual":
                    p.add("ERROR", tid, f"depends on manual task {d}, so the swarm would wait for the owner; "
                          f"build against a stand-in and let {d} replace it")
                if t["status"] in ("done", "review", "in-progress") and tasks[d]["status"] != "done":
                    p.add("WARN", tid, f"is {t['status']} but its dependency {d} is {tasks[d]['status']}")
        whole = [r for r in t["requirements"] if ":" not in r]
        if len(whole) > MAX_WHOLE_REQS:
            p.add("WARN", tid, f"covers {len(whole)} whole requirements; consider splitting it")
        if not t["owns"] and t["kind"] != "manual":
            p.add("WARN", tid, "owns no paths; an executor needs to know where it may write")
        for path in t["owns"] + t["shares"]:
            if path.startswith("/") or "*" in path or ".." in path.split("/"):
                p.add("ERROR", tid, f"path {path!r} must be relative to the project root, with no globs or ..")
        headings = {h.lower(): v for h, v in t["sections"].items()}
        for s in REQUIRED_SECTIONS:
            if s.lower() not in headings:
                p.add("WARN", tid, f"no '## {s}' section")
        empty = [s for s in REQUIRED_SECTIONS if s.lower() in headings and not headings[s.lower()].strip()]
        if empty and len(empty) == sum(s.lower() in headings for s in REQUIRED_SECTIONS):
            unwritten.append(tid)
            continue  # nothing below means anything for a body that isn't written yet
        if empty:
            p.add("WARN", tid, f"empty section{'s' if len(empty) > 1 else ''}: {', '.join(empty)}")
        mentioned = set(re.findall(r"\b[A-Z]{2,4}-\d+\b|\bU\d+\b", t["body"]))
        for ref in t["requirements"]:
            rid = ref.partition(":")[0]
            if rid not in mentioned:
                p.add("WARN", tid, f"{rid} is in requirements but the body never mentions it")
        read_first = next((v for k, v in t["sections"].items() if k.lower() == "read first"), "")
        for m in PATH_RE.finditer(read_first):
            if not (plan.root / m.group(1)).exists():
                p.add("WARN", tid, f"Read first cites {m.group(1)}, which doesn't exist")
    if unwritten:
        p.add("WARN", "", f"{len(unwritten)} task bodies aren't written yet (headings only): {', '.join(unwritten)}")
    if not g.ok:
        return
    live = g.tasks
    owners = [(tid, path) for tid in sorted(live, key=id_sort) for path in live[tid]["owns"]]
    for i, (ta, pa) in enumerate(owners):
        for tb, pb in owners[i + 1:]:
            if ta != tb and overlap(pa, pb) and not g.ordered(ta, tb):
                p.add("ERROR", "", f"{ta} owns {pa} and {tb} owns {pb}, and they may run at the same "
                      f"time; give the path one owner, or order them")
    for tid in sorted(live, key=id_sort):
        for path in live[tid]["shares"]:
            owning = [o for o in live if o != tid and any(overlap(path, op) for op in live[o]["owns"])]
            if any(overlap(path, op) for op in live[tid]["owns"]):
                p.add("WARN", tid, f"both owns and shares {path}")
            if not owning:
                p.add("ERROR", tid, f"shares {path}, which no task owns; the task that creates it should own it")
                continue
            if not any(o in g.anc[tid] for o in owning):
                p.add("ERROR", tid, f"shares {path}, but its owner ({', '.join(owning)}) doesn't come "
                      f"before {tid}, so the file may not exist yet")
            for o in owning:
                if not g.ordered(o, tid):
                    p.add("ERROR", tid, f"shares {path} while {o} owns it, and they may run at the same time")


def hotspots(g: Graph) -> tuple[list[tuple[str, list[str], list[str]]], list[tuple[str, list[str]]]]:
    shared: dict[str, list[str]] = {}
    for tid in sorted(g.tasks, key=id_sort):
        for path in g.tasks[tid]["shares"]:
            shared.setdefault(path, []).append(tid)
    rows = []
    for path, sharers in sorted(shared.items()):
        owners = [o for o in sorted(g.tasks, key=id_sort)
                  if any(overlap(path, op) for op in g.tasks[o]["owns"])]
        rows.append((path, owners, sharers))
    handovers: dict[str, list[str]] = {}
    for tid in g.order:
        for path in g.tasks[tid]["owns"]:
            handovers.setdefault(path, [])
    for path in list(handovers):
        hs = [t for t in g.order if any(overlap(path, op) for op in g.tasks[t]["owns"])]
        handovers[path] = hs
    hand = [(path, ts) for path, ts in sorted(handovers.items()) if len(ts) > 1]
    return rows, hand


def stale_snapshot(spec: Spec, plan: Plan) -> dict | None:
    gj = plan.dir / "graph.json"
    if not gj.exists():
        return None
    try:
        old = json.loads(gj.read_text(encoding="utf-8")).get("snapshot")
    except (json.JSONDecodeError, OSError):
        return None
    return old


def snapshot_changes(old: dict, spec: Spec) -> dict:
    cur = spec.snapshot()
    oreq, creq = old.get("requirements", {}), cur["requirements"]
    ch = {
        "added": sorted([r for r in creq if r not in oreq], key=id_sort),
        "removed": sorted([r for r in oreq if r not in creq], key=id_sort),
        "dropped": sorted([r for r in creq if r in oreq and creq[r]["dropped"] and not oreq[r]["dropped"]],
                          key=id_sort),
        "restored": sorted([r for r in creq if r in oreq and oreq[r]["dropped"] and not creq[r]["dropped"]],
                           key=id_sort),
    }
    flipped = set(ch["dropped"]) | set(ch["restored"])
    ch["changed"] = sorted([r for r in creq if r in oreq and r not in flipped
                            and creq[r]["hash"] != oreq[r]["hash"]], key=id_sort)
    op, cp = old.get("parts", {}), cur["parts"]
    ch["parts_added"] = sorted(k for k in cp if k not in op and k.split(":")[0] in oreq)
    ch["parts_removed"] = sorted(k for k in op if k not in cp and k.split(":")[0] in creq)
    ch["parts_changed"] = sorted(k for k in cp if k in op and cp[k] != op[k])
    osec, csec = old.get("sections", {}), cur["sections"]
    ch["sections_changed"] = sorted(k for k in csec if k in osec and csec[k] != osec[k])
    ch["sections_added"] = sorted(k for k in csec if k not in osec)
    ch["sections_removed"] = sorted(k for k in osec if k not in csec)
    od, cd = old.get("decisions", {}), cur["decisions"]
    ch["decisions"] = sorted([(k, od.get(k), cd.get(k)) for k in set(od) | set(cd) if od.get(k) != cd.get(k)],
                             key=lambda x: id_sort(x[0]))
    of, cf = old.get("findings", {}), cur["findings"]
    ch["findings"] = sorted((k, of.get(k, ""), cf.get(k, "")) for k in set(of) | set(cf)
                            if of.get(k, "").startswith("open") != cf.get(k, "").startswith("open"))
    ch["any"] = any(ch[k] for k in ch if k != "any")
    return ch


def run_check(root: Path, scope_arg: str | None, quiet: bool = False):
    spec = Spec(root)
    p = Problems()
    plan = Plan(root, p)
    if not plan.tasks:
        p.add("ERROR", "", "no tasks in plan/tasks/")
    g = Graph(plan, p)
    check_tasks(spec, plan, g, p)
    scope = parse_scope(scope_arg, plan)
    cov = coverage(spec, plan, p, scope)
    old = stale_snapshot(spec, plan)
    if old:
        ch = snapshot_changes(old, spec)
        if ch["any"]:
            n = len(ch["added"]) + len(ch["removed"]) + len(ch["changed"]) + len(ch["dropped"])
            p.add("WARN", "", f"the specs changed since the plan was rendered ({n} requirements, "
                  f"{len(ch['sections_changed'])} sections); run `plan.py diff` and update the plan")
    if not quiet:
        summary(spec, plan, g, cov, scope)
        p.show()
        e, w = p.count("ERROR"), p.count("WARN")
        print(f"\n{e} errors, {w} warnings" + ("" if e else " — the plan passes"))
    return spec, plan, g, p, cov, scope


def parse_scope(arg: str | None, plan: Plan) -> set[str] | None:
    if arg:
        return {s.strip().upper() for s in arg.split(",") if s.strip()}
    gj = plan.dir / "graph.json"
    if gj.exists():
        try:
            s = json.loads(gj.read_text(encoding="utf-8")).get("scope")
            return set(s) if s else None
        except (json.JSONDecodeError, OSError):
            return None
    return None


def summary(spec: Spec, plan: Plan, g: Graph, cov: dict, scope) -> None:
    live = plan.live()
    by_status = {}
    for t in live.values():
        by_status[t["status"]] = by_status.get(t["status"], 0) + 1
    st = ", ".join(f"{k} {v}" for k, v in sorted(by_status.items(), key=lambda kv: STATUSES.index(kv[0])
                                                if kv[0] in STATUSES else 99))
    print(f"plan: {len(live)} tasks ({st or 'none'})"
          + (f", {len(plan.tasks) - len(live)} dropped" if len(plan.tasks) > len(live) else ""))
    if g.ok and live:
        chain, w = g.critical_path()
        total = sum(g.weight(t) for t in g.tasks)
        print(f"waves: {len(g.waves)}, widest {max(len(w_) for w_ in g.waves)}; "
              f"critical path {' → '.join(chain)} (weight {w} of {total})")
    covered = cov["in_scope"] - len(cov["uncovered"]) - len(cov["partial"])
    print(f"coverage: {covered} of {cov['in_scope']} requirements"
          + (f" in scope {','.join(sorted(scope))}" if scope else "") + "\n")


# --- render -------------------------------------------------------------------------------------


def md_escape(s: str) -> str:
    return s.replace("|", "\\|")


def render(root: Path, accept: bool, scope_arg: str | None) -> int:
    spec, plan, g, p, cov, scope = run_check(root, scope_arg, quiet=True)
    if not plan.tasks:
        print("render stopped: plan/tasks/ has no task files yet")
        return 1
    structural = [x for x in p.items if x[0] == "ERROR" and ("cycle" in x[2] or "frontmatter" in x[2]
                                                                or "doesn't exist" in x[2])]
    if not g.ok or structural:
        p.show(("ERROR",))
        print("\nrender stopped: fix the errors above first")
        return 1
    today = date.today().isoformat()
    old = stale_snapshot(spec, plan)
    snap = spec.snapshot()
    if old and not accept:
        ch = snapshot_changes(old, spec)
        if ch["any"]:
            print("The specs changed since the last render; keeping the old snapshot so `plan.py diff` "
                  "still shows the changes. Update the plan, then render with --accept-spec-changes.\n")
            snap = old
    chain, cp_weight = g.critical_path()
    rem = g.remaining()
    rows, hand = hotspots(g)
    tasks_out = []
    for tid in sorted(g.tasks, key=id_sort):
        t = g.tasks[tid]
        tasks_out.append({
            "id": tid, "title": t["title"], "kind": t["kind"], "status": t["status"],
            "milestone": t["milestone"], "size": t["size"], "wave": g.level[tid],
            "remaining": rem[tid], "depends_on": t["depends_on"], "dependents": g.dependents[tid],
            "requirements": t["requirements"], "owns": t["owns"], "shares": t["shares"], "file": t["file"],
        })
    dropped = sorted([tid for tid, t in plan.tasks.items() if t["status"] == "dropped"], key=id_sort)
    graph = {
        "generated": today,
        "scope": sorted(scope) if scope else None,
        "tasks": tasks_out,
        "dropped_tasks": dropped,
        "waves": g.waves,
        "critical_path": chain,
        "coverage": {"in_scope": cov["in_scope"], "uncovered": cov["uncovered"], "partial": cov["partial"]},
        "snapshot": snap,
    }
    (plan.dir / "graph.json").write_text(json.dumps(graph, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    (plan.dir / "coverage.md").write_text(coverage_md(spec, cov, scope, today), encoding="utf-8")
    readme = plan.dir / "README.md"
    if readme.exists():
        text = readme.read_text(encoding="utf-8")
        blocks = {
            "status": status_block(plan, g, cov, chain, cp_weight),
            "waves": waves_block(g),
            "critical": critical_block(g, chain, cp_weight),
            "graph": graph_block(g),
            "hotspots": hotspots_block(rows, hand),
            "open": open_block(spec, cov),
        }
        missing = []
        for name in README_BLOCKS:
            text, found = replace_block(text, name, blocks[name])
            if not found:
                missing.append(name)
        readme.write_text(text, encoding="utf-8")
        if missing:
            print("README.md has no markers for: " + ", ".join(
                f"<!-- plan:{n} --><!-- /plan:{n} -->" for n in missing))
    else:
        print("plan/README.md doesn't exist yet; wrote graph.json and coverage.md only")
    print(f"rendered plan/graph.json, plan/coverage.md and the README blocks ({len(g.tasks)} tasks, "
          f"{len(g.waves)} waves)")
    e = p.count("ERROR")
    if e:
        print(f"{e} errors remain; run `plan.py check`")
    return 0


def replace_block(text: str, name: str, content: str) -> tuple[str, bool]:
    start, end = f"<!-- plan:{name} -->", f"<!-- /plan:{name} -->"
    pat = re.compile(re.escape(start) + r".*?" + re.escape(end), re.S)
    if not pat.search(text):
        return text, False
    return pat.sub(lambda _: f"{start}\n{content}\n{end}", text, count=1), True


def status_block(plan: Plan, g: Graph, cov: dict, chain: list[str], w: int) -> str:
    live = plan.live()
    counts = {s: sum(1 for t in live.values() if t["status"] == s) for s in STATUSES if s != "dropped"}
    parts = [f"{v} {k}" for k, v in counts.items() if v]
    covered = cov["in_scope"] - len(cov["uncovered"]) - len(cov["partial"])
    return (f"_Generated {date.today().isoformat()}._ **{len(live)} tasks:** {', '.join(parts)}. "
            f"**{len(g.waves)} waves**, critical path weight {w}. "
            f"**Coverage:** {covered} of {cov['in_scope']} requirements.")


def waves_block(g: Graph) -> str:
    out = ["| Wave | Task | Title | Kind | Size | Status | Depends on |", "|---|---|---|---|---|---|---|"]
    for n, wave in enumerate(g.waves, start=1):
        for tid in wave:
            t = g.tasks[tid]
            link = f"[{tid}]({t['file'].removeprefix('plan/')})"
            out.append(f"| {n} | {link} | {md_escape(t['title'])} | {t['kind']} | {t['size']} | "
                       f"{t['status']} | {', '.join(t['depends_on']) or '–'} |")
    return "\n".join(out)


def critical_block(g: Graph, chain: list[str], w: int) -> str:
    total = sum(g.weight(t) for t in g.tasks)
    steps = " → ".join(f"{t} ({g.tasks[t]['size'] or '?'})" for t in chain)
    return (f"{steps}\n\nWeight {w} of {total} (S = 1, M = 2, L = 3). Start these first, and keep them "
            f"small: every task on this path delays the end by its full length.")


def graph_block(g: Graph) -> str:
    out = ["```mermaid", "flowchart LR"]
    for tid in sorted(g.tasks, key=id_sort):
        t = g.tasks[tid]
        title = t["title"].replace('"', "'")
        if len(title) > 34:
            title = title[:33].rstrip() + "…"
        cls = {"done": "done", "in-progress": "active", "review": "active", "blocked": "blocked"}.get(t["status"])
        if t["kind"] == "manual":
            cls = "manual" if cls is None else cls
        out.append(f'  {tid}["{tid} {title}"]' + (f":::{cls}" if cls else ""))
    for a, b in g.reduced_edges():
        out.append(f"  {a} --> {b}")
    out += [
        "  classDef done fill:#d8f0dc,stroke:#2f7d3b",
        "  classDef active fill:#fff1c2,stroke:#a07800",
        "  classDef blocked fill:#f8d7da,stroke:#a12a35",
        "  classDef manual stroke-dasharray: 4 3",
        "```",
        "",
        "Edges implied by others are left out. Green: done; yellow: in progress or review; "
        "red: blocked; dashed: manual.",
    ]
    return "\n".join(out)


def hotspots_block(rows, hand) -> str:
    out = []
    if rows:
        out += ["Files that several tasks touch (`shares`). The owner creates the file; sharers add to it, "
                "so expect merge conflicts here, and merge in wave order.", "",
                "| Path | Owner | Shared by |", "|---|---|---|"]
        out += [f"| `{path}` | {', '.join(owners) or '–'} | {', '.join(sharers)} |" for path, owners, sharers in rows]
    else:
        out.append("No task shares a path with another.")
    if hand:
        out += ["", "Paths that pass from one task to a later one (`owns` of ordered tasks):", "",
                "| Path | Tasks, in order |", "|---|---|"]
        out += [f"| `{path}` | {' → '.join(ts)} |" for path, ts in hand]
    return "\n".join(out)


def open_block(spec: Spec, cov: dict) -> str:
    who = {}
    for rid, ts in cov["whole"].items():
        who.setdefault(rid, set()).update(ts)
    for ref, ts in cov["parts"].items():
        who.setdefault(ref.partition(":")[0], set()).update(ts)
    out = []
    open_dec = [d for d in spec.decisions if d["state"] == "open"]
    if open_dec:
        out += ["**Open spec decisions** (specs/decisions.md):", ""]
        out += [f"- **{d['id']}** {d['title']} — {d['detail']}" for d in open_dec]
    of = spec.open_findings()
    if of:
        out += ["", "**Open review findings** (specs/reviews/), and the tasks whose requirements they touch:", ""]
        for f in of:
            ts = sorted({t for rid in f["ids"] for t in who.get(rid, ())}, key=id_sort)
            out.append(f"- **{f['log']} {f['id']}** ({f['severity']}) {f['title']} → "
                       f"{', '.join(ts) if ts else 'no task'}")
    proposals = [r["id"] for r in spec.reqs.values() if r["proposal"] and not r["dropped"]]
    if proposals:
        out += ["", f"**Requirements marked (proposal)** ({len(proposals)}): built as written unless the "
                f"user decides otherwise: {', '.join(sorted(proposals, key=id_sort))}."]
    return "\n".join(out) if out else "No open decisions or findings."


def coverage_md(spec: Spec, cov: dict, scope, today: str) -> str:
    who: dict[str, list[str]] = {}
    for rid, ts in cov["whole"].items():
        who.setdefault(rid, []).extend(ts)
    part_who: dict[str, list[str]] = {}
    for ref, ts in cov["parts"].items():
        part_who.setdefault(ref.partition(":")[0], []).append(f"{'+'.join(ts)} ({ref.partition(':')[2]})")
    out = ["# Requirement coverage", "",
           f"_Generated by `plan.py render` on {today}; don't edit by hand._", "",
           f"Scope: {', '.join(sorted(scope)) if scope else 'all requirements'}. "
           f"In scope: {cov['in_scope']}; uncovered: {len(cov['uncovered'])}; partly covered: {len(cov['partial'])}.",
           ""]
    by_file: dict[str, list[dict]] = {}
    for r in spec.reqs.values():
        by_file.setdefault(r["file"], []).append(r)
    for f in sorted(by_file):
        out += [f"## {f}", "", "| ID | Flags | Tasks |", "|---|---|---|"]
        for r in sorted(by_file[f], key=lambda r: id_sort(r["id"])):
            flags = [x for x, on in (("dropped", r["dropped"]), ("proposal", r["proposal"]),
                                     ("decision needed", r["decision_needed"])) if on]
            if scope and r["prefix"] not in scope:
                flags.append("out of scope")
            tasks = sorted(set(who.get(r["id"], [])), key=id_sort)
            cell = ", ".join(tasks)
            if r["id"] in part_who:
                cell = (cell + "; " if cell else "") + "parts: " + "; ".join(sorted(part_who[r["id"]]))
            if not cell and not r["dropped"]:
                cell = "**none**"
            out.append(f"| {r['id']} | {', '.join(flags)} | {cell} |")
        out.append("")
    return "\n".join(out)


# --- other commands ----------------------------------------------------------------------------


def cmd_extract(root: Path, json_out: str | None) -> int:
    spec = Spec(root)
    reqs = list(spec.reqs.values())
    by_prefix: dict[str, list[dict]] = {}
    for r in reqs:
        by_prefix.setdefault(r["prefix"], []).append(r)
    files = sorted({r["file"] for r in reqs})
    print(f"specs: {len(files)} files with requirements; {len(reqs)} requirements")
    for pfx in sorted(by_prefix):
        rs = by_prefix[pfx]
        dropped = [r["id"] for r in rs if r["dropped"]]
        print(f"  {pfx:4} {len(rs):3}" + (f"  dropped: {', '.join(dropped)}" if dropped else ""))
    props = [r["id"] for r in reqs if r["proposal"] and not r["dropped"]]
    print(f"proposals: {len(props)} requirements carry '(proposal)' (in their text or section heading)")
    umbrella = sorted([r for r in reqs if len(r["parts"]) >= 4], key=lambda r: -len(r["parts"]))
    if umbrella:
        print("requirements with many parts (split them with ID:part when one task can't hold them all):")
        for r in umbrella:
            print(f"  {r['id']:7} {len(r['parts']):3} parts  ({r['file']}:{r['line']})")
    if spec.duplicates:
        print("DUPLICATE definitions: " + "; ".join(spec.duplicates))
    if spec.dangling:
        seen = {}
        for d in spec.dangling:
            seen.setdefault(d["id"], []).append(f"{d['file']}:{d['line']}")
        print("mentioned but never defined: " + "; ".join(f"{k} ({', '.join(v[:3])})" for k, v in seen.items()))
    dec_open = [d for d in spec.decisions if d["state"] == "open"]
    print(f"decisions: {sum(1 for d in spec.decisions if d['state'] == 'resolved')} resolved, {len(dec_open)} open"
          + "".join(f"\n  {d['id']} {d['title']}" for d in dec_open))
    of = spec.open_findings()
    print(f"open review findings: {len(of)}" + "".join(
        f"\n  {f['log']} {f['id']} ({f['severity']}) {f['title']}  [{', '.join(f['ids'])}]" for f in of))
    if json_out:
        data = {"generated": date.today().isoformat(), "requirements": reqs, "sections": spec.sections,
                "decisions": spec.decisions, "findings": spec.findings, "dangling": spec.dangling}
        Path(json_out).write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"wrote {json_out}")
    return 0


def cmd_show(root: Path, ids: list[str]) -> int:
    spec = Spec(root)
    p = Problems()
    plan = Plan(root, p)
    rc = 0
    for raw in ids:
        rid, _, key = raw.partition(":")
        rid = rid.upper()
        r = spec.reqs.get(rid)
        if not r:
            print(f"{rid}: not defined in specs/")
            rc = 1
            continue
        part = next((x for x in r["parts"] if x["key"] == key), None) if key else None
        if key and not part:
            print(f"{rid} has no part {key!r}; its parts are: {', '.join(spec.parts_of(rid)) or 'none'}")
            rc = 1
            continue
        flags = [x for x, on in (("DROPPED", r["dropped"]), ("proposal", r["proposal"]),
                                 ("decision needed", r["decision_needed"])) if on]
        tasks = sorted({tid for tid, t in plan.live().items() for ref in t["requirements"]
                        if ref.partition(":")[0] == rid and (not key or ref.partition(":")[2] in ("", key))},
                       key=id_sort)
        print(f"{rid}{':' + key if key else ''}  {r['file']}:{r['line']}  "
              f"({r['section'].split('#', 1)[1] or 'top'})" + (f"  [{', '.join(flags)}]" if flags else ""))
        if part:
            print(f"  {part['text']}")
            if part["refs"]:
                print(f"  refers to: {', '.join(part['refs'])}")
            print(f"  tasks: {', '.join(tasks) if tasks else 'none'}\n")
            continue
        print(f"  {r['text']}")
        if r["refs"]:
            print(f"  refers to: {', '.join(r['refs'])}")
        if r["decisions"]:
            print(f"  decisions: {', '.join(r['decisions'])}")
        if r["parts"]:
            print("  parts: " + ", ".join(f"{rid}:{x['key']}" for x in r["parts"]))
        print(f"  tasks: {', '.join(tasks) if tasks else 'none'}\n")
    return rc


def cmd_hints(root: Path) -> int:
    spec, plan, g, p, cov, _ = run_check(root, None, quiet=True)
    if not g.ok:
        p.show(("ERROR",))
        return 1
    holders: dict[str, set[str]] = {}
    for rid, ts in cov["whole"].items():
        holders.setdefault(rid, set()).update(ts)
    for ref, ts in cov["parts"].items():
        holders.setdefault(ref.partition(":")[0], set()).update(ts)
    pairs: dict[tuple[str, str], list[str]] = {}
    for tid in sorted(g.tasks, key=id_sort):
        for held in sorted(set(g.tasks[tid]["requirements"]), key=id_sort):
            rid, _, key = held.partition(":")
            r = spec.reqs.get(rid)
            if not r:
                continue
            part = next((x for x in r["parts"] if x["key"] == key), None) if key else None
            for ref in (part["refs"] if part else [] if key else r["refs"]):
                for other in holders.get(ref, ()):
                    if other != tid and other in g.tasks and not g.ordered(tid, other):
                        pairs.setdefault(tuple(sorted((tid, other), key=id_sort)), []).append(f"{held}→{ref}")
    if not pairs:
        print("No spec cross-references between tasks that may run at the same time.")
        return 0
    print("Spec cross-references between tasks that may run at the same time. For each pair, decide:\n"
          "add a dependency, put what they share in contracts.md, or note why it's incidental.\n")
    for (a, b), refs in sorted(pairs.items(), key=lambda kv: (-len(kv[1]), id_sort(kv[0][0]))):
        uniq = sorted(set(refs))
        print(f"{a} ({g.tasks[a]['title']}) ↔ {b} ({g.tasks[b]['title']}): {len(uniq)}")
        print("   " + ", ".join(uniq[:12]) + (" …" if len(uniq) > 12 else ""))
    return 0


def cmd_index(root: Path) -> int:
    """Every task's frontmatter and the first paragraph of its Outcome, for helper subagents' briefs."""
    p = Problems()
    plan = Plan(root, p)
    if not plan.tasks:
        print("plan/tasks/ has no task files yet")
        return 1
    out = ["| Task | Title | Kind | Size | Status | Depends on | Owns | Shares | Requirements | Outcome |",
           "|---|---|---|---|---|---|---|---|---|---|"]
    for tid in sorted(plan.tasks, key=id_sort):
        t = plan.tasks[tid]
        outcome = next((v for k, v in t["sections"].items() if k.lower() == "outcome"), "").strip()
        outcome = norm(outcome.split("\n\n")[0]) if outcome else ""
        cells = [tid, t["title"], t["kind"], t["size"], t["status"], ", ".join(t["depends_on"]) or "–",
                 ", ".join(t["owns"]) or "–", ", ".join(t["shares"]) or "–",
                 ", ".join(t["requirements"]) or "–", outcome or "–"]
        out.append("| " + " | ".join(md_escape(c) for c in cells) + " |")
    print("\n".join(out))
    p.show(("ERROR",))
    return 0


def cmd_ready(root: Path) -> int:
    spec, plan, g, p, cov, _ = run_check(root, None, quiet=True)
    if not g.ok:
        p.show(("ERROR",))
        return 1
    rem = g.remaining()
    live = g.tasks
    done = {t for t in live if live[t]["status"] == "done"}
    ready = [t for t in live if live[t]["status"] == "todo" and all(d in done for d in g.deps[t])]
    ready.sort(key=lambda t: (-rem[t], id_sort(t)))
    swarm = [t for t in ready if live[t]["kind"] != "manual"]
    manual = [t for t in ready if live[t]["kind"] == "manual"]
    if swarm:
        print(f"Ready for executors ({len(swarm)}), most urgent first:")
        for t in swarm:
            x = live[t]
            print(f"  {t}  {x['size'] or '?'}  wave {g.level[t]}  remaining {rem[t]:2}  {x['title']}")
            print(f"        {x['file']}  owns: {', '.join(x['owns']) or '–'}"
                  + (f"  shares: {', '.join(x['shares'])}" if x["shares"] else ""))
    else:
        print("No task is ready for an executor.")
    if manual:
        print("\nReady for the owner (manual): " + ", ".join(f"{t} {live[t]['title']}" for t in manual))
    for st in ("in-progress", "review", "blocked"):
        ts = sorted([t for t in live if live[t]["status"] == st], key=id_sort)
        if ts:
            print(f"\n{st}: " + ", ".join(f"{t} {live[t]['title']}" for t in ts))
    waiting = [t for t in live if live[t]["status"] == "todo" and t not in ready]
    print(f"\n{len(done)} of {len(live)} done; {len(waiting)} waiting on dependencies.")
    errs = p.count("ERROR")
    if errs:
        print(f"Note: `plan.py check` reports {errs} errors.")
    return 0


def cmd_status(root: Path, tid: str, status: str) -> int:
    if status not in STATUSES:
        die(f"status must be one of {', '.join(STATUSES)}")
    p = Problems()
    plan = Plan(root, p)
    t = plan.tasks.get(tid)
    if not t:
        die(f"no task {tid}")
    path = root / t["file"]
    lines = path.read_text(encoding="utf-8").split("\n")
    end = next(i for i in range(1, len(lines)) if lines[i].strip() == "---")
    for i in range(1, end):
        if re.match(r"^status:", lines[i]):
            old = lines[i].split(":", 1)[1].strip()
            lines[i] = f"status: {status}"
            path.write_text("\n".join(lines), encoding="utf-8")
            print(f"{tid}: {old} → {status}")
            return 0
    die(f"{t['file']} has no status line")
    return 2


def cmd_diff(root: Path) -> int:
    spec = Spec(root)
    p = Problems()
    plan = Plan(root, p)
    old = stale_snapshot(spec, plan)
    if not old:
        print("No snapshot yet: plan/graph.json is missing. Render the plan first.")
        return 1
    ch = snapshot_changes(old, spec)
    if not ch["any"]:
        print(f"No spec changes since the snapshot of {old.get('date', '?')}.")
        return 0
    live = plan.live()

    def holders(rid: str, key: str = "") -> str:
        ts = []
        for tid, t in sorted(live.items(), key=lambda kv: id_sort(kv[0])):
            for ref in t["requirements"]:
                r, _, k = ref.partition(":")
                if r == rid and (not key or not k or k == key):
                    ts.append(f"{tid} ({t['status']})")
                    break
        return ", ".join(ts) if ts else "no task"

    def citing(section_key: str) -> str:
        file, _, anc = section_key.partition("#")
        ts = []
        for tid, t in sorted(live.items(), key=lambda kv: id_sort(kv[0])):
            rf = next((v for k, v in t["sections"].items() if k.lower() == "read first"), "")
            for m in PATH_RE.finditer(rf):
                if m.group(1) == file and (not m.group(2) or m.group(2)[1:] == anc):
                    ts.append(f"{tid} ({t['status']})")
                    break
        return ", ".join(ts) if ts else "no task cites it"

    print(f"Spec changes since the plan's snapshot of {old.get('date', '?')}:\n")
    for label, key in (("added", "added"), ("changed", "changed"), ("dropped", "dropped"),
                       ("restored", "restored"), ("removed", "removed")):
        for rid in ch[key]:
            r = spec.reqs.get(rid)
            where = f"{r['file']}:{r['line']}" if r else "(no longer defined)"
            print(f"  {label:8} {rid:7} {where:32} → {holders(rid)}")
    for label, key in (("added", "parts_added"), ("removed", "parts_removed"), ("changed", "parts_changed")):
        for ref in ch[key]:
            rid, _, k = ref.partition(":")
            print(f"  part {label:8} {ref:40} → {holders(rid, k)}")
    if ch["sections_changed"] or ch["sections_added"] or ch["sections_removed"]:
        print("\nSections whose text changed (they may hold rules outside any requirement ID):")
        for key in ch["sections_changed"]:
            print(f"  changed  {key}  → {citing(key)}")
        for key in ch["sections_added"]:
            print(f"  added    {key}")
        for key in ch["sections_removed"]:
            print(f"  removed  {key}")
    if ch["decisions"]:
        print("\nDecisions:")
        for k, a, b in ch["decisions"]:
            print(f"  {k}: {a or 'new'} → {b or 'gone'}")
    if ch["findings"]:
        print("\nReview findings whose open state changed:")
        for k, a, b in ch["findings"]:
            print(f"  {k}: {a or 'new'} → {b or 'gone'}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", help="project root (the directory with specs/ and docs/); default: search upwards")
    sub = ap.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("extract")
    e.add_argument("--json", dest="json_out")
    s = sub.add_parser("show")
    s.add_argument("ids", nargs="+")
    c = sub.add_parser("check")
    c.add_argument("--scope", help="comma-separated requirement prefixes, e.g. MOD,POL,STA,TST")
    sub.add_parser("hints")
    sub.add_parser("index")
    r = sub.add_parser("render")
    r.add_argument("--accept-spec-changes", action="store_true")
    r.add_argument("--scope")
    sub.add_parser("ready")
    st = sub.add_parser("status")
    st.add_argument("id")
    st.add_argument("status")
    sub.add_parser("diff")
    a = ap.parse_args()
    root = find_root(a.root)
    if a.cmd == "extract":
        return cmd_extract(root, a.json_out)
    if a.cmd == "show":
        return cmd_show(root, a.ids)
    if a.cmd == "check":
        *_, p, _, _ = run_check(root, a.scope)
        return 1 if p.count("ERROR") else 0
    if a.cmd == "hints":
        return cmd_hints(root)
    if a.cmd == "index":
        return cmd_index(root)
    if a.cmd == "render":
        return render(root, a.accept_spec_changes, a.scope)
    if a.cmd == "ready":
        return cmd_ready(root)
    if a.cmd == "status":
        return cmd_status(root, a.id.upper(), a.status)
    if a.cmd == "diff":
        return cmd_diff(root)
    return 2


def find_root(arg: str | None) -> Path:
    if arg:
        root = Path(arg).resolve()
        if not (root / "specs").is_dir():
            die(f"{root} has no specs/ directory")
        return root
    here = Path.cwd().resolve()
    for cand in (here, *here.parents):
        if (cand / "specs").is_dir() and (cand / "docs").is_dir():
            return cand
    die("can't find the project root (a directory with specs/ and docs/); pass --root")
    return here


if __name__ == "__main__":
    sys.exit(main())
