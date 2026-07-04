export default async function handler(req, res) {
  if (req.method !== "POST") {
    res.status(405).json({ error: "Method not allowed" });
    return;
  }

  const upstreamBaseUrl = (process.env.ARXIVIST_UPSTREAM_API_BASE_URL ?? "").replace(/\/$/, "");
  if (!upstreamBaseUrl) {
    res.status(500).json({ error: "ARXIVIST_UPSTREAM_API_BASE_URL is not configured" });
    return;
  }

  try {
    const upstream = await fetch(`${upstreamBaseUrl}/search`, {
      method: "POST",
      headers: {
        "content-type": req.headers["content-type"] ?? "application/json"
      },
      body: typeof req.body === "string" ? req.body : JSON.stringify(req.body ?? {})
    });
    const text = await upstream.text();
    res.status(upstream.status);
    res.setHeader("content-type", upstream.headers.get("content-type") ?? "application/json");
    res.send(text);
  } catch {
    res.status(502).json({ error: "Search API unavailable" });
  }
}
