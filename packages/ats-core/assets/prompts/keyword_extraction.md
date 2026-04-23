Extract, categorize, and rank Applicant Tracking System (ATS) keywords from the provided job description based on the following algorithmic rules.

**Task 1: Extraction and Categorization**
Parse the input text and extract all relevant keywords into these specific categories:

- **Hard Skills and Tools:** Programming languages, software platforms, technical workflows, and methodologies.
- **Soft Skills and Competencies:** Leadership, cross-functional collaboration, problem-solving, and communication abilities.
- **Industry-Specific Terminology:** Sector-specific jargon, performance metrics, and regulatory frameworks (e.g., HIPAA, KPIs, return on investment).
- **Certifications and Credentials:** Required degrees, professional licenses, and formal certifications.
- **Job Titles and Seniority:** Exact role titles and leadership scope indicators (e.g., strategic planning, change management).
  Whenever a term includes an acronym, extract both the fully spelled-out form and its abbreviation (e.g., "Search Engine Optimization (SEO)" or "Customer Relationship Management (CRM)").

**Task 2: Semantic Grouping**
Because modern ATS platforms utilize Natural Language Processing (NLP) to evaluate semantic equivalents, group contextually related terms together within your extracted lists. For example, cluster variations like "project management," "managing projects," and "program coordination" as a single semantic entity.

**Task 3: Algorithmic Ranking**
Rank the extracted keyword clusters in descending order of importance based on standard ATS scoring parameters:

- **Keyword Frequency:** Assign the highest weight to terms that appear multiple times throughout the job description.
- **Document Location:** Prioritize terms explicitly located under mandatory headers such as "Requirements," "Qualifications," or "Preferred Experience".
- **Skill Weighting:** Rank technical requirements, hard skills, and tools significantly higher than soft skills, as technical competencies typically account for 40% to 60% of the total ATS relevance score.

**System Constraints & Output Format:**

- Only extract terms explicitly stated or directly semantically implied within the provided text. Do not hallucinate or inject external industry buzzwords.
- Mirror the exact language used by the employer whenever possible, as some systems enforce literal matching.
- Format the final output strictly as a structured JSON object containing the categorized and ranked keyword clusters.
