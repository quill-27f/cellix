#!/usr/bin/env python3
"""A scripted language server for the tour's code-intelligence scenes: symbols, workspace
symbols, signature help and inlay hints, read from the text with regular expressions, so
the pictures don't wait on a real server's indexing."""
import json, os, re, sys

texts = {}


def read():
    length = 0
    while True:
        header = sys.stdin.buffer.readline()
        if not header:
            sys.exit()
        header = header.strip()
        if not header:
            break
        if header.lower().startswith(b"content-length:"):
            length = int(header.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(length))


def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def where(uri, n, start, end):
    return {"uri": uri, "range": {"start": {"line": n, "character": start},
                                  "end": {"line": n, "character": end}}}


def symbols(uri, text):
    out, owner, block = [], "", ""
    for n, line in enumerate(text.split("\n")):
        found = None
        if m := re.match(r"\s*pub struct (\w+)", line):
            owner, block = m.group(1), ""
            found = (m, 23, "")
        elif m := re.match(r"impl(?:<\w+>)? (\w+)", line):
            block = "impl " + m.group(1) + ("<T>" if "<T>" in line else "")
            out.append({"name": block, "kind": 19, "location": where(uri, n, 0, len(line))})
            continue
        elif m := re.match(r"\s*(?:pub )?fn (\w+)", line):
            inside = line.startswith(" ") and block
            found = (m, 6 if inside else 12, block if inside else "")
        elif (m := re.match(r"\s*(?:pub )?const (\w+)", line)):
            found = (m, 14, "")
        elif owner and (m := re.match(r"    (\w+): ", line)) and not block:
            found = (m, 8, owner)
        elif line.startswith("}"):
            owner = block = ""
        if found:
            m, kind, container = found
            out.append({"name": m.group(1), "kind": kind, "containerName": container,
                        "location": where(uri, n, m.start(1), m.end(1))})
    return out


def every_file():
    for folder, _, names in os.walk("."):
        if "target" in folder or ".git" in folder:
            continue
        for name in names:
            if name.endswith(".rs"):
                path = os.path.abspath(os.path.join(folder, name))
                uri = "file://" + path
                if uri not in texts:
                    with open(path) as f:
                        texts[uri] = f.read()
    return texts.items()


TYPES = [(r"Queue::new\(", "Queue<Job>"), (r"load_jobs\(", "Vec<Job>"), (r"\d+;", "usize")]
PARAMETERS = {"Queue::new": ["limit"], "push": ["item"], "log": ["level", "message"]}


def hints(text):
    out = []
    for n, line in enumerate(text.split("\n")):
        if m := re.search(r"let (?:mut )?(\w+) = (.*)", line):
            for pattern, kind in TYPES:
                if re.match(pattern, m.group(2)):
                    out.append({"position": {"line": n, "character": m.end(1)},
                                "label": ": " + kind, "kind": 1})
                    break
        if m := re.search(r"for (\w+) in", line):
            out.append({"position": {"line": n, "character": m.end(1)}, "label": ": Job", "kind": 1})
        if m := re.search(r"if let Err\((\w+)\)", line):
            out.append({"position": {"line": n, "character": m.end(1)}, "label": ": Full", "kind": 1})
        for name, names in PARAMETERS.items() if "fn " not in line else []:
            for call in re.finditer(re.escape(name) + r"\(", line):
                at = call.end()
                for parameter in names:
                    out.append({"position": {"line": n, "character": at},
                                "label": parameter + ":", "kind": 2, "paddingRight": True})
                    comma = line.find(",", at)
                    if comma < 0:
                        break
                    at = comma + 2
    return out


SIGNATURE = {
    "label": "pub fn insert(&mut self, index: usize, value: T)",
    "documentation": "Inserts an element at index within the deque, shifting all elements "
    "with indices greater than or equal to index towards the back.",
    "parameters": [{"label": "&mut self"},
                   {"label": "index: usize", "documentation": "Where it goes, from the front."},
                   {"label": "value: T", "documentation": "the element to put at index."}],
}

while True:
    message = read()
    method, ident = message.get("method"), message.get("id")
    params = message.get("params", {})
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": ident, "result": {"capabilities": {}}})
    elif method == "textDocument/didOpen":
        texts[params["textDocument"]["uri"]] = params["textDocument"]["text"]
    elif method == "textDocument/didChange":
        texts[params["textDocument"]["uri"]] = params["contentChanges"][-1]["text"]
    elif method == "textDocument/documentSymbol":
        uri = params["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": ident, "result": symbols(uri, texts.get(uri, ""))})
    elif method == "workspace/symbol":
        query = params["query"].lower()
        found = [s for uri, text in every_file() for s in symbols(uri, text)
                 if s["kind"] != 19 and query in s["name"].lower()]
        send({"jsonrpc": "2.0", "id": ident, "result": found})
    elif method == "textDocument/inlayHint":
        uri = params["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": ident, "result": hints(texts.get(uri, ""))})
    elif method == "textDocument/signatureHelp":
        position = params["position"]
        line = texts.get(params["textDocument"]["uri"], "").split("\n")[position["line"]]
        before = line[:position["character"]]
        if "insert(" in before and ")" not in before[before.rfind("insert("):]:
            active = 1 + before[before.rfind("insert("):].count(",")
            send({"jsonrpc": "2.0", "id": ident, "result": {
                "signatures": [SIGNATURE], "activeSignature": 0, "activeParameter": active}})
        else:
            send({"jsonrpc": "2.0", "id": ident, "result": None})
    elif ident is not None:
        send({"jsonrpc": "2.0", "id": ident, "result": None})
