#!/usr/bin/env python3
"""Cloud read/receipt client. Credential comes only from an environment secret."""
import argparse
import hashlib
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request


def retrieve(base, path, token):
    request = urllib.request.Request(base + path, headers={"Authorization": "Bearer " + token} if token else {})
    # Do not forward credentials through redirects.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            return None
    with urllib.request.build_opener(NoRedirect).open(request, timeout=35) as response:
        data = response.read(65537)
    if len(data) > 65536:
        raise ValueError("Oversized remote response")
    return json.loads(data)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default=os.environ.get("DIRECT_REMOTE_READ_URL"))
    parser.add_argument("--proxy-credential", action="store_true", help="Use Claude cloud API credential injection; no token is available to this process")
    args = parser.parse_args()
    parsed = urllib.parse.urlsplit(args.url or "")
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise ValueError("A credential-free HTTPS bridge origin is required")
    token = os.environ.get("DIRECT_REMOTE_READ_TOKEN", "")
    if not args.proxy_credential and (len(token) != 64 or any(c not in "0123456789abcdef" for c in token)):
        raise ValueError("DIRECT_REMOTE_READ_TOKEN environment secret unavailable")
    if args.proxy_credential:
        token = None
    base = args.url.rstrip("/")
    manifest = retrieve(base, "/v1/manifest", token)
    if not 1 <= len(manifest["issues"]) <= 8:
        raise ValueError("Invalid manifest")
    contexts = []
    receipts = []
    for item in manifest["issues"]:
        context = retrieve(base, "/v1/context/" + urllib.parse.quote(item["key"], safe=""), token)
        received_hash = context["fingerprint"]
        canonical = {k: v for k, v in context.items() if k not in ("fingerprint", "observed_at")}
        calculated = hashlib.sha256(json.dumps(canonical, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
        if context["key"] != item["key"] or calculated != received_hash or context["version"] != item["version"] or received_hash != item["fingerprint"]:
            raise ValueError("Context changed during retrieval; rerun the read before implementing")
        contexts.append(context)
        receipts.append({k: context[k] for k in ("key", "version", "fingerprint")})
    # Reread the manifest after all briefs. No receipt is printed for a mixed version set.
    if retrieve(base, "/v1/manifest", token)["issues"] != manifest["issues"]:
        raise ValueError("Context changed during retrieval; rerun the read before implementing")
    print(json.dumps({"manifest": manifest, "contexts": contexts, "receipt": receipts}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as error:
        print("Remote read failed: HTTP " + str(error.code) + "; coordinator must repair access or scope.", file=sys.stderr)
        sys.exit(1)
    except (ValueError, KeyError, TypeError, urllib.error.URLError):
        print("Remote read failed: missing/invalid access, unavailable context, or changed version; coordinator must repair the handoff.", file=sys.stderr)
        sys.exit(1)
