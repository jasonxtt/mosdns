"""Conservative, query-specific evidence for learned no-address suppression."""


def explain(left, right, qtype, audit, trace, rules, sizes=None, limit=None):
    if (left["rcode"] != 0 or right["rcode"] != 0
            or left["has_answer"] == right["has_answer"] or not trace.get("policy_valid")):
        return None
    if left["tc"] != right["tc"]:
        suppressed, passed = (left, right) if right["has_answer"] else (right, left)
        passed_size = sizes[1 if right["has_answer"] else 0] if sizes else None
        if (suppressed["tc"] or not passed["tc"] or passed_size is None or limit is None
                or passed_size > max(512, limit)):
            return None
    ident = audit.get("trace_id")
    entries = trace.get("entries", [])
    if not ident or any(e.get("trace_id") != ident for e in entries):
        return None
    if not entries or entries[-1].get("msg") != "dns query completed":
        return None
    for rule in rules:
        if qtype != rule["qtype"]:
            continue
        steps = [e for e in entries if e.get("logger_name") == rule["sequence"]]
        marker = f'anonymous_match(fast_mark: {rule["mark"]})'
        matches = [i for i, e in enumerate(steps) if e.get("matcher_name") == marker]
        # Reentry/branches with multiple uses of the mark are ambiguous.
        if len(matches) != 1:
            continue
        index = matches[0]
        if not any(e.get("plugin_name") == rule["mapper"] for e in steps[:index]):
            continue
        hit = steps[index].get("match_result")
        if hit is False and right["has_answer"]:
            return dict(rule, matched=False, trace_id=ident, proof="learned-suppression-miss")
        guards = steps[index + 1:index + 4]
        if (hit is True and not right["has_answer"] and audit.get("domain_set") == rule["label"]
                and len(guards) == 3
                and guards[0].get("matcher_name") == f'anonymous_match(qtype: {qtype})'
                and guards[0].get("match_result") is True
                and guards[1].get("matcher_name") == f'anonymous_match({rule["switch"]}: \'A\')'
                and guards[1].get("match_result") is True
                and guards[2].get("plugin_name") == "anonymous_exec(reject: 0)"):
            return dict(rule, matched=True, trace_id=ident, proof="learned-suppression-hit")
    return None
