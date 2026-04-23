Extract the primary job posting from the provided HTML.

**Extraction Rules:**

1. Identify the primary job posting on the page. If multiple exist, extract only the main one.
2. Extract the job title and the full job description (including company, location, employment type, responsibilities, requirements, and benefits).
3. Exclude all non-posting content: navigation, footers, cookie banners, related jobs, ads, application forms, recruiter marketing, and legal boilerplate.
4. Preserve the exact original wording and language. Do not paraphrase, summarize, translate, or invent content.
5. Format the description as clean Markdown:
   - Use `##` headings for logical sections, preferring the source's exact labels.
   - Use bullet points for lists.
   - Preserve original paragraph breaks.
   - Exclude all HTML tags, tables, images, and links.
6. If the page does not contain a job posting, extract the page's main content on a best-effort basis. Do not refuse the request.
