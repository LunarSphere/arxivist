# Arxivist Agent Notes

- Do not edit anything under `Legacy/`; it is reference material for the original learning project.
- Build the new system component by component. Do not replace the whole architecture in one shot.
- Keep concise design comments near important boundaries so the implementation is useful for learning.
- Prefer Rust for application services and AWS-managed services for production infrastructure.
- Keep local-development adapters available before adding AWS adapters, so each component can be tested without cloud access.
