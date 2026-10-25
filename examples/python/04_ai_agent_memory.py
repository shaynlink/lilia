"""04_ai_agent_memory.py - AI Agent conversation session & memory store with LiliaDB.

Requirements:
- Run lilia-daemon with HTTP enabled:
    cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret
- Run this script:
    python3 examples/python/04_ai_agent_memory.py
"""

import os
import sys
import time
from typing import Any, Dict, List, Optional

from liliadb import LiliaClient

ENDPOINT = os.environ.get("LILIA_ENDPOINT", "http://127.0.0.1:4444")
TOKEN = os.environ.get("LILIA_TOKEN", "dev-secret")


class AgentMemoryStore:
    """Manages AI agent long-term facts, working memory, and conversation history."""

    def __init__(self, client: LiliaClient):
        self.client = client
        self.session_space = "agent_sessions"
        self.facts_space = "agent_facts"

    def save_fact(self, user_id: str, fact_key: str, fact_value: str) -> None:
        """Store a long-term user preference or fact."""
        self.client.kv.set(self.facts_space, f"{user_id}:{fact_key}", fact_value)

    def get_fact(self, user_id: str, fact_key: str) -> Optional[str]:
        """Retrieve a stored user preference or fact."""
        entry = self.client.kv.get(self.facts_space, f"{user_id}:{fact_key}")
        return entry.value if entry else None

    def append_message(self, session_id: str, role: str, content: str) -> None:
        """Append a message to conversation history with optimistic concurrency."""
        doc = self.client.json.get(self.session_space, session_id)
        if doc is None:
            history = [{"role": role, "content": content, "ts": time.time()}]
            self.client.json.put(self.session_space, session_id, {"session_id": session_id, "messages": history})
        else:
            data = doc.value
            data.setdefault("messages", []).append({"role": role, "content": content, "ts": time.time()})
            # Atomic update with version check
            self.client.json.put(self.session_space, session_id, data, if_version=int(doc.version))

    def get_recent_history(self, session_id: str, max_messages: int = 10) -> List[Dict[str, Any]]:
        """Retrieve the most recent messages for context generation."""
        doc = self.client.json.get(self.session_space, session_id)
        if not doc or "messages" not in doc.value:
            return []
        return doc.value["messages"][-max_messages:]

    def search_sessions_with_keywords(self, keyword: str) -> List[Dict[str, Any]]:
        """Find sessions containing a specific word using LiliaDB SQL."""
        query = f"""
            SELECT id, value
            FROM lilia_json
            WHERE space = 'agent_sessions'
              AND value LIKE '%{keyword}%'
        """
        res = self.client.sql(query)
        return res.to_dicts()


def main() -> None:
    client = LiliaClient(endpoint=ENDPOINT, token=TOKEN)

    try:
        client.health()
    except Exception as exc:
        print(f"Error connecting to daemon at {ENDPOINT}: {exc}")
        print("\nPlease start the daemon with:")
        print("  cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret\n")
        sys.exit(1)

    memory = AgentMemoryStore(client)
    user_id = "user_42"
    session_id = f"sess_{user_id}_001"

    print("\n--- 1. Store Agent Long-Term Facts (KV Store) ---")
    memory.save_fact(user_id, "preferred_language", "Python")
    memory.save_fact(user_id, "theme", "dark")
    print(f"Fact 'preferred_language' for {user_id}: {memory.get_fact(user_id, 'preferred_language')}")

    print("\n--- 2. Record Conversation History (JSON Document) ---")
    memory.append_message(session_id, "user", "How can I deploy my Rust microservice?")
    memory.append_message(session_id, "assistant", "You can package it in a scratch Docker container or deploy as binary.")
    memory.append_message(session_id, "user", "Does LiliaDB provide an HTTP gateway for my agent?")
    memory.append_message(session_id, "assistant", "Yes, lilia-daemon includes a high-performance local loopback HTTP REST API!")

    recent = memory.get_recent_history(session_id, max_messages=2)
    print(f"Last 2 conversation turns in session {session_id}:")
    for msg in recent:
        print(f"  [{msg['role'].upper()}]: {msg['content']}")

    print("\n--- 3. Search Past Interactions with SQL ---")
    results = memory.search_sessions_with_keywords("HTTP")
    print(f"Found {len(results)} sessions mentioning 'HTTP':")
    for r in results:
        print(f"  Session ID: {r['id']}")

    print("\n--- 4. Cleanup ---")
    client.json.delete("agent_sessions", session_id)
    client.kv.delete("agent_facts", f"{user_id}:preferred_language")
    client.kv.delete("agent_facts", f"{user_id}:theme")
    print("Cleaned up agent memory demo data.")


if __name__ == "__main__":
    main()
