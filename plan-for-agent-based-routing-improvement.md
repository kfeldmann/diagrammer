# Goal

- Edge segments must not lie directly over (and parallel to) another edge segment or a group outline
- Edge segments running parallel to another segment or a group outline must not be closer than 5 pixels to the other segment
- Edge corners must not touch another edge corner, or come within 5 pixels of one
- Edge segments may cross another segment or a group outline perpendicularly
- Edge segments must not pass through a node
- Edge segments may pass through a group
- Routes that use fewer segments should be prioritized over routes with more segments

# Strategy

- You may use up to 4 subagents simultaneously
- You can define the agents: system prompt, prompt, model, thinking level...
- Devise your own strategy for using agents to develop a routing algorithm that best meets the goals listed above
