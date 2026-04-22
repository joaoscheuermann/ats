# Early returns and guard clauses

This convention prefers the **guard clause** pattern (early returns) over deep nesting of `if/else`. Handle edge cases, errors, and invalid states at the top of a function so the happy path stays at low indentation.

## Instructions

1. **Handle edge cases first**: Check invalid inputs, errors, or bail-out conditions at the beginning of the function.
2. **Return immediately**: After a guard resolves control flow, return (or throw) immediately. Do not use an `else` block after a `return`.
3. **Flatter is better**: Keep primary logic minimally indented. If you are three or more levels deep in `if` statements, refactor toward early returns.
4. **Exceptions**: Small `if/else` for simple assignments can stay if clearer than a guard; prefer ternaries for trivial cases.

## Examples

### Bad (deep nesting)

```javascript
function processUser(user) {
  if (user) {
    if (user.isActive) {
      if (user.hasPermission) {
        saveUser(user)
        return 'Saved'
      } else {
        return 'No permission'
      }
    } else {
      return 'Inactive'
    }
  } else {
    throw new Error('No user')
  }
}
```

### Good (early returns)

```javascript
function processUser(user) {
  if (!user) throw new Error('No user')
  if (!user.isActive) return 'Inactive'
  if (!user.hasPermission) return 'No permission'

  saveUser(user)
  return 'Saved'
}
```

## Constraints

- **Avoid** `else` immediately after a block that ends with `return`, `throw`, or `break`.
- **Avoid** nesting the main logic inside a large outer `if`.
- **Prefer** inverting conditions to handle the negative case first, then continue on the main level.
