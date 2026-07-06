const allowedRoutes = new Map([
  ["GET /health", true],
  ["POST /search", true],
  ["GET /agent/health", true],
  ["POST /agent/search", true]
]);

function getPath(queryPath) {
  const parts = Array.isArray(queryPath) ? queryPath : [queryPath].filter(Boolean);
  return `/${parts.join("/")}`;
}

function getBody(req) {
  if (req.method === "GET" || req.method === "HEAD") {
    return undefined;
  }

  if (typeof req.body === "string") {
    return req.body;
  }

  if (req.body == null) {
    return undefined;
  }

  return JSON.stringify(req.body);
}

function upstreamForPath(path) {
  if (path.startsWith("/agent/")) {
    return {
      baseUrl: (process.env.ARXIVIST_UPSTREAM_AGENT_API_BASE_URL ?? "").replace(/\/$/, ""),
      missingMessage: "ARXIVIST_UPSTREAM_AGENT_API_BASE_URL is not configured",
      unavailableMessage: "Agent API unavailable"
    };
  }

  return {
    baseUrl: (process.env.ARXIVIST_UPSTREAM_API_BASE_URL ?? "").replace(/\/$/, ""),
    missingMessage: "ARXIVIST_UPSTREAM_API_BASE_URL is not configured",
    unavailableMessage: "Search API unavailable"
  };
}

export default async function handler(req, res) {
  const path = getPath(req.query.path);
  const routeKey = `${req.method} ${path}`;
  const upstream = upstreamForPath(path);

  if (!allowedRoutes.has(routeKey)) {
    res.status(404).json({ error: "Not found" });
    return;
  }

  if (!upstream.baseUrl) {
    res.status(500).json({ error: upstream.missingMessage });
    return;
  }

  try {
    const upstreamResponse = await fetch(`${upstream.baseUrl}${path}`, {
      method: req.method,
      headers: {
        "content-type": req.headers["content-type"] ?? "application/json"
      },
      body: getBody(req)
    });
    const text = await upstreamResponse.text();
    const contentType = upstreamResponse.headers.get("content-type") ?? "application/json";

    res.status(upstreamResponse.status);
    res.setHeader("content-type", contentType);
    res.send(text);
  } catch (error) {
    res.status(502).json({ error: upstream.unavailableMessage });
  }
}
