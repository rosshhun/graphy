from flask import Flask, jsonify

app = Flask(__name__)

@app.route("/health")
def health_check():
    """Health endpoint - should be alive (decorated route)."""
    return jsonify({"status": "ok"})

@app.route("/api/stats")
def api_stats():
    """Stats endpoint - should be alive (decorated route)."""
    data = compute_stats()
    return jsonify(data)

def compute_stats():
    """Helper called by api_stats - should be alive (has callers)."""
    return {"count": 42, "avg": 3.14}

def unused_helper():
    """Truly dead code - no callers, no decorators, no references."""
    return "never called"

def another_dead_function():
    """Also dead - no references anywhere."""
    pass
