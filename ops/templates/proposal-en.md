# Proposal Template — English

> Each proposal must be **customized** to the specific job. Generic boilerplate gets ignored.
> Target length: **150-250 words**. Longer proposals have lower response rates on Upwork.

---

## Structure (5 paragraphs, ~50 words each)

### P1 — Hook (proves you read the brief)
> Reference one *specific detail* from their post. Show you understand the actual problem, not just the keywords.

> Example: "You mentioned that your current Zapier flow breaks when the Notion API rate-limits — that's a classic webhook backpressure issue, and the fix isn't just retry logic."

### P2 — Credibility (1-2 sentences, no resume dump)
> One concrete past project that maps to their problem. Numbers if possible.

> Example: "I built a high-concurrency draw system handling 10K+ req/s in production for a gaming firm, and currently lead CI/CD for an SI team building iOS / IoT systems."

### P3 — Approach (your plan, in 3-4 bullets)
> Show you've thought about *how* you'd do it. This is what separates senior from junior bidders.

```
- Step 1: ...
- Step 2: ...
- Step 3: ...
- Deliverable: ...
```

### P4 — Differentiator (your AI angle)
> One paragraph on why you ship faster.

> Example: "I use Claude as a force-multiplier — meaning I deliver in days what typical contractors quote in weeks, without sacrificing code quality. You get senior judgment with AI-accelerated execution."

### P5 — Call to action
> One specific question that requires them to reply.

> Example: "Quick question before I quote: does the existing system have a staging environment I can test against, or am I working straight against prod?"

---

## Full example (use as starting point)

```
Hi [Name],

You mentioned the existing Stripe webhook handler is dropping events under load — that's almost always a queue/idempotency issue rather than a Stripe-side problem, and it's fixable in a focused 2-3 day sprint.

I've shipped high-concurrency systems before (built a real-time draw engine handling 10K+ req/s for a gaming company) and I currently lead CI/CD infrastructure for an SI team in Taiwan working on iOS, IoT, and 5G systems.

Here's how I'd tackle this:
- Audit your current webhook handler + log a week of traffic to confirm the failure pattern
- Add a durable queue (SQS / Redis Streams depending on your stack) with idempotency keys
- Add observability (Sentry + custom dashboard) so you'll see issues before customers report them
- Deliverable: PR with tests + deployment guide + 2 weeks of bug-fix support

What makes me different: I work with Claude as a co-engineer, which means I deliver in days what typical contractors quote in weeks — but you still get senior architectural judgment, not just AI-generated code.

Before I quote: is the webhook handler in your main monolith, or is it already a separate service? That'll change the deployment plan.

— [Your Name]
```

---

## Anti-patterns (DON'T do these)

- ❌ "Dear Sir/Madam, I am very interested in your project..."
- ❌ "I have 10+ years experience in [list of every tech]..."
- ❌ "Please check my profile for portfolio"
- ❌ "I can finish this in 1 day for $50" (race to bottom)
- ❌ Quoting a price without asking clarifying questions first
- ❌ Using emoji 🚀 ✅ 💯 (looks like spam template)
