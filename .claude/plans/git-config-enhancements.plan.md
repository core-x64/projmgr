# Plan: Git Config Enhensions and Keybinding Verification

**Source PRD**: User request in conversation
**Selected Milestone**: Enhance git functionality and config options
**Complexity**: Medium

## Summary
Verify existing git functionality and keybindings are working correctly, then extend the configuration system with additional options as requested by the user. The user wants more configurable options in ~/.config/unit-projman/unit.cfg beyond just auto_git, and verification that all mentioned keybindings exist and work properly.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `src/ui.rs:170-182` | AppConfig struct with descriptive field names |
| Errors | `src/ui.rs:189-209` | load_config() with graceful fallback to defaults |
| Tests | `src/ui.rs:1578-1583` | Conditional execution based on config values |

## Files to Change
| File | Action | Why |
|---|---|---|
| `src/ui.rs` | UPDATE | Extend AppConfig struct with new options |
| `src/ui.rs` | UPDATE | Enhance load_config() to handle new options |
| `src/ui.rs` | UPDATE | Make GitHub link visibility configurable |
| `src/ui.rs` | UPDATE | Add validation for config values |
| `src/data.rs` | UPDATE | If needed to pass config to provider methods |

## Tasks
### Task 1: Extend AppConfig with new options
- **Action**: Add fields for github_visibility, auto_commit_initial, default_branch_name, etc.
- **Mirror**: AppConfig struct pattern from lines 170-182
- **Validate**: Check struct definition and default implementation

### Task 2: Update load_config() to handle new options
- **Action**: Modify TOML deserialization to include new fields with sensible defaults
- **Mirror**: load_config() function pattern from lines 189-209
- **Validate**: Verify config file creation and reading works with new fields

### Task 3: Make GitHub link visibility configurable
- **Action**: Update action_github_link() to use config.github_visibility instead of hardcoded --private
- **Mirror**: action_github_link() function from lines 1897-1946
- **Validate**: Test that different visibility options work correctly

### Task 4: Add config validation
- **Action**: Add validation for config values (e.g., github_visibility must be valid)
- **Mirror**: Error handling patterns throughout src/ui.rs
- **Validate**: Ensure invalid config values fall back to defaults gracefully

### Task 5: Verify existing keybindings work
- **Action**: Confirm all mentioned keybindings (gc, se, ca, a in templates) are functional
- **Mirror**: Keybinding handling in handle_dashboard_key() and handle_project_options_key()
- **Validate**: Test each keybinding produces expected behavior

### Task 6: Document new config options
- **Action**: Update comments and help text to describe new configuration options
- **Mirror**: Existing comments and status messages
- **Validate**: Clear documentation of what each option does

## Validation
```bash
# Verify config file creation and loading
mkdir -p ~/.config/unit-projman
echo '[ ]' > ~/.config/unit-projman/unit.cfg  # Empty config to test defaults
# Run projmgr and check it uses defaults

# Test with custom config
echo 'auto_git = true
github_visibility = "public"
default_editor = "vim"' > ~/.config/unit-projman/unit.cfg
# Run projmgr and verify settings are applied

# Test GitHub linking with different visibility settings
# (Would need actual GitHub token for full test)
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| Config file corruption | Low | Load/save with error handling and fallback to defaults |
| Invalid config values | Medium | Validate values and use safe fallbacks |
| GitHub API changes | Medium | Abstract gh calls behind helper functions |
| Keybinding conflicts | Low | Use established prefix system (g, s, c) |

## Acceptance
- [ ] All existing keybindings (gc, se, ca, a) verified working
- [ ] Config file supports auto_git, github_visibility, default_editor, etc.
- [ ] GitHub link uses configurable visibility (public/private/internal)
- [ ] Invalid config values fall back to sensible defaults
- [ ] New config options are documented in code/comments