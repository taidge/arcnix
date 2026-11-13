"""Translate legacy UI selectors while leaving fixture names and paths intact."""
from pathlib import Path
import re

CATALOG = [line.split("\t", 1) for line in (Path(__file__).resolve().parents[1] / "crates/arcnix/assets/translations.tsv").read_text(encoding="utf-8").splitlines() if "\t" in line]
EXACT = dict(CATALOG)


def tr(value, _depth=0):
    if not isinstance(value, str) or _depth > 4:
        return value
    if value in EXACT:
        return EXACT[value]
    for source, target in sorted(CATALOG, key=lambda pair: len(re.sub(r"\{[^}]*\}", "", pair[0])), reverse=True):
        if "{" not in source:
            continue
        fields = re.findall(r"\{([^}]*)\}", source)
        pattern = "(.*?)".join(re.escape(part) for part in re.split(r"\{[^}]*\}", source))
        match = re.fullmatch(pattern, value, re.DOTALL)
        if not match:
            continue
        values = {}
        auto = 0
        for field, captured in zip(fields, match.groups()):
            key = field.split(":", 1)[0]
            if not key:
                key = str(auto)
                auto += 1
            values[key] = tr(captured, _depth + 1) if key in ["label","direction","location","error","e","message","warning"] else captured
        auto = 0
        def replace(found):
            nonlocal auto
            key = found.group(1).split(":", 1)[0]
            if not key:
                key = str(auto)
                auto += 1
            return values.get(key, "")
        return re.sub(r"\{([^}]*)\}", replace, target)
    return value


def install():
    from playwright.sync_api import Page, Locator, LocatorAssertions
    for cls in [Page, Locator]:
        for method in ["get_by_role", "get_by_text"]:
            original = getattr(cls, method)
            def wrapped(self, *args, _original=original, _method=method, **kwargs):
                if _method == "get_by_text" and args:
                    args = (tr(args[0]), *args[1:])
                if "name" in kwargs:
                    kwargs["name"] = tr(kwargs["name"])
                return _original(self, *args, **kwargs)
            setattr(cls, method, wrapped)
    for method in ["to_have_text", "to_contain_text", "not_to_have_text", "not_to_contain_text"]:
        original = getattr(LocatorAssertions, method)
        def wrapped(self, expected, *args, _original=original, **kwargs):
            if isinstance(expected, list):
                expected = [tr(value) for value in expected]
            else:
                translated = tr(expected)
                if translated == expected and isinstance(expected,str) and len(expected)>4:
                    matches=[target for source,target in CATALOG if "{" not in source and source.startswith(expected)]
                    if matches:
                        translated=min(matches,key=len)
                expected = translated
            return _original(self, expected, *args, **kwargs)
        setattr(LocatorAssertions, method, wrapped)
