#!/usr/bin/env python3
"""Persistent RapidOCR worker using a length-prefixed JSON protocol."""

import base64
import json
import re
import struct
import sys
import unicodedata
from difflib import SequenceMatcher

import cv2
import numpy as np
from rapidocr import RapidOCR


def read_message():
    header = sys.stdin.buffer.read(4)
    if not header:
        return None
    size = struct.unpack("<I", header)[0]
    payload = sys.stdin.buffer.read(size)
    if len(payload) != size:
        raise EOFError("truncated OCR request")
    return json.loads(payload)


def write_message(payload):
    encoded = json.dumps(payload, ensure_ascii=False).encode()
    sys.stdout.buffer.write(struct.pack("<I", len(encoded)))
    sys.stdout.buffer.write(encoded)
    sys.stdout.buffer.flush()


def bounds(box):
    xs = [point[0] for point in box]
    ys = [point[1] for point in box]
    return min(xs), min(ys), max(xs), max(ys)


def select_text(result):
    if result is None or not result.txts:
        raise ValueError("no text detected")

    entries = []
    for box, text, score in zip(result.boxes, result.txts, result.scores):
        text = text.strip()
        if score < 0.72 or sum(character.isalpha() for character in text) < 2:
            continue
        left, top, right, bottom = bounds(box)
        entries.append(
            {
                "text": text,
                "score": float(score),
                "left": left,
                "top": top,
                "right": right,
                "bottom": bottom,
                "center": (top + bottom) / 2,
                "height": max(1, bottom - top),
            }
        )
    if not entries:
        raise ValueError("no confident text detected")

    entries.sort(key=lambda entry: (entry["center"], entry["left"]))
    lines = []
    for entry in entries:
        if lines and abs(entry["center"] - lines[-1]["center"]) <= max(
            entry["height"], lines[-1]["height"]
        ) * 0.55:
            line = lines[-1]
            line["entries"].append(entry)
            line["left"] = min(line["left"], entry["left"])
            line["right"] = max(line["right"], entry["right"])
            line["top"] = min(line["top"], entry["top"])
            line["bottom"] = max(line["bottom"], entry["bottom"])
            line["center"] = (line["top"] + line["bottom"]) / 2
            line["height"] = max(1, line["bottom"] - line["top"])
        else:
            lines.append({**entry, "entries": [entry]})

    for line in lines:
        line["entries"].sort(key=lambda entry: entry["left"])
        line["text"] = " ".join(entry["text"] for entry in line["entries"])

    blocks = []
    for line in lines:
        if blocks:
            previous = blocks[-1]["lines"][-1]
            gap = line["top"] - previous["bottom"]
            horizontal_overlap = min(line["right"], previous["right"]) - max(
                line["left"], previous["left"]
            )
            aligned = abs(line["left"] - previous["left"]) <= max(
                line["height"], previous["height"]
            ) * 2
            if gap <= max(line["height"], previous["height"]) * 1.35 and (
                horizontal_overlap > 0 or aligned
            ):
                blocks[-1]["lines"].append(line)
                continue
        blocks.append({"lines": [line]})

    def block_score(block):
        text = " ".join(line["text"] for line in block["lines"])
        letters = sum(character.isalpha() for character in text)
        words = len(text.split())
        sentence = 18 if text.endswith((".", "!", "?")) else 0
        return letters + min(words, 18) * 2 + len(block["lines"]) * 8 + sentence

    selected = max(blocks, key=block_score)
    selected_entries = [entry for line in selected["lines"] for entry in line["entries"]]
    text = normalize_text(" ".join(line["text"] for line in selected["lines"]))
    confidence = sum(entry["score"] for entry in selected_entries) / len(selected_entries)
    return {
        "text": text,
        "confidence": confidence * 100,
        "line_count": len(selected["lines"]),
        "word_confidences": [entry["score"] * 100 for entry in selected_entries],
    }


def normalize_text(text):
    words = []
    for word in text.split():
        letters = "".join(character for character in word if character.isalpha())
        if (
            len(letters) >= 2
            and letters.isupper()
            and letters not in {"DNA", "ONA", "HP", "PP", "EXP"}
        ):
            continue
        plain = "".join(
            character
            for character in unicodedata.normalize("NFKD", letters)
            if character.isascii()
        ).lower()
        if plain == "ona":
            word = word.replace(letters, "DNA")
        elif 6 <= len(plain) <= 8 and SequenceMatcher(None, plain, "pokemon").ratio() >= 0.75:
            word = word.replace(letters, "Pokémon")
        words.append(word)
    text = " ".join(words)
    text = re.sub(r"\b([A-Za-z]+) ([A-Za-z]) (\2[A-Za-z]+)\b", r"\1 \3", text)
    terminal = max(text.rfind("."), text.rfind("!"), text.rfind("?"))
    if terminal >= 0 and len(text[terminal + 1 :].split()) <= 3:
        text = text[: terminal + 1]
    return text


def main():
    engine = RapidOCR(
        params={
            "Global.log_level": "warning",
            "Global.use_cls": False,
            "Global.min_height": 12,
        }
    )
    while request := read_message():
        try:
            encoded = base64.b64decode(request["png"])
            image = cv2.imdecode(np.frombuffer(encoded, np.uint8), cv2.IMREAD_COLOR)
            if image is None:
                raise ValueError("invalid PNG")
            write_message({"ok": True, **select_text(engine(image))})
        except Exception as error:
            write_message({"ok": False, "error": str(error)})


if __name__ == "__main__":
    main()
