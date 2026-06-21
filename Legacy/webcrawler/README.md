The crawler is responsible for 
1. Scraping the web async
2. creating the sqlite database
3. populating (pages, page_contents, links, page_assets tables)

***Notes***
I'm pretty satisfied with the current crawler but it would benefit from respecting robots.txt 
rust has the robotparser and the robotstxt(port of googles robot parser) crate so try one of those.
