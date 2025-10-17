#!/usr/bin/env python3
"""Extrait les diagnostics d'erreur de `cargo check` dans un format facile à partager.

Le script exécute `cargo check --message-format=json`, filtre uniquement les
messages de niveau "error" et les affiche avec un séparateur `***` pour simplifier
la copie dans une discussion.

Usage de base :
    scripts/cargo_error_report.py --all-features

Tous les arguments passés au script après les options propres sont relayés à
`cargo check`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from typing import Iterable, List, Optional


@dataclass
class CargoError:
    code: Optional[str]
    file: Optional[str]
    line: Optional[int]
    rendered: str


def parse_args(argv: Optional[Iterable[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Collecte les erreurs de cargo check")
    parser.add_argument(
        "cargo_args",
        nargs=argparse.REMAINDER,
        help="Arguments supplémentaires passés à `cargo check`",
    )
    return parser.parse_args(argv)


def run_cargo_check(extra_args: List[str]) -> tuple[int, List[CargoError], str]:
    cmd = ["cargo", "check", "--message-format=json"] + extra_args
    process = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    errors: List[CargoError] = []

    assert process.stdout is not None  # for type checkers
    for line in process.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue

        message = payload.get("message")
        if not message or message.get("level") != "error":
            continue

        rendered = message.get("rendered") or message.get("message") or "(message vide)"
        code = None
        if isinstance(message.get("code"), dict):
            code = message["code"].get("code")

        file_name = None
        line_number = None
        for span in message.get("spans", []):
            if span.get("is_primary"):
                file_name = span.get("file_name")
                line_number = span.get("line_start")
                break

        errors.append(CargoError(code=code, file=file_name, line=line_number, rendered=rendered))

    stdout_rest = process.stdout.read()
    if stdout_rest:
        # On ignore le reste, il ne contient normalement pas de diagnostics.
        pass

    stderr = process.stderr.read() if process.stderr else ""
    return_code = process.wait()
    return return_code, errors, stderr


def print_errors(return_code: int, errors: List[CargoError], stderr: str) -> None:
    if not errors and return_code == 0:
        print("✅ Aucune erreur détectée.")
        return

    if not errors and return_code != 0:
        print("❌ `cargo check` a échoué mais aucune erreur n'a été extraite.")
        if stderr:
            print("--- stderr ---")
            print(stderr.strip())
        return

    for idx, err in enumerate(errors, start=1):
        header = [f"Erreur #{idx}"]
        if err.code:
            header.append(f"code {err.code}")
        if err.file and err.line:
            header.append(f"{err.file}:{err.line}")
        elif err.file:
            header.append(err.file)
        print(" - ".join(header))
        print("***")
        print(err.rendered.rstrip())
        print("***\n")

    if return_code == 0:
        print("ℹ️ `cargo check` s'est terminé avec succès malgré ces erreurs (probablement dues à une compilation incrémentale).")
    else:
        print("❌ `cargo check` a échoué.")
        if stderr.strip():
            print("--- stderr ---")
            print(stderr.strip())


def main() -> int:
    args = parse_args()
    return_code, errors, stderr = run_cargo_check(args.cargo_args)
    print_errors(return_code, errors, stderr)
    return 0 if return_code == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
