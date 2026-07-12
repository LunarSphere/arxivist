export default async function handler(req, res) {
  if (req.method !== "GET") {
    res.status(405).json({ error: "Method not allowed" });
    return;
  }

  const upstreamBaseUrl = (process.env.ARXIVIST_UPSTREAM_API_BASE_URL ?? "").replace(/\/$/, "");
  if (!upstreamBaseUrl) {
    res.status(500).json({ error: "ARXIVIST_UPSTREAM_API_BASE_URL is not configured" });
    return;
  }

  try {
    const headers = {};
    if (process.env.ARXIVIST_UPSTREAM_SHARED_SECRET) {
      headers["x-arxivist-proxy-secret"] = process.env.ARXIVIST_UPSTREAM_SHARED_SECRET;
    }
    const upstream = await fetch(`${upstreamBaseUrl}/health`, { headers });
    const text = await upstream.text();
    res.status(upstream.status);
    res.setHeader("content-type", upstream.headers.get("content-type") ?? "application/json");
    res.send(text);
  } catch {
    res.status(502).json({ error: "Search API unavailable" });
  }
}
