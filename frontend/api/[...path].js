const allowedRoutes = new Map([
  ["GET /health", true],
  ["POST /search", true]
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

export default async function handler(req, res) {
  const upstreamBaseUrl = (process.env.ARXIVIST_UPSTREAM_API_BASE_URL ?? "").replace(/\/$/, "");
  const path = getPath(req.query.path);
  const routeKey = `${req.method} ${path}`;

  if (!allowedRoutes.has(routeKey)) {
    res.status(404).json({ error: "Not found" });
    return;
  }

  if (!upstreamBaseUrl) {
    res.status(500).json({ error: "ARXIVIST_UPSTREAM_API_BASE_URL is not configured" });
    return;
  }

  try {
    const upstream = await fetch(`${upstreamBaseUrl}${path}`, {
      method: req.method,
      headers: {
        "content-type": req.headers["content-type"] ?? "application/json"
      },
      body: getBody(req)
    });
    const text = await upstream.text();
    const contentType = upstream.headers.get("content-type") ?? "application/json";

    res.status(upstream.status);
    res.setHeader("content-type", contentType);
    res.send(text);
  } catch (error) {
    res.status(502).json({ error: "Search API unavailable" });
  }
}
