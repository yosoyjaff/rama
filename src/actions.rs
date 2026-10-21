//! The actions the shell answers for.
//!
//! One logical command is modelled once, as an Action: the key binding, the
//! menu item and any button that triggers it dispatch the same value, so they
//! cannot drift apart. A feature defines its own actions in its own module the
//! same way.
//!
//! Every one here has a trigger in the UI: an Action with no way to reach it
//! is behaviour nobody can find and everybody has to delete. Quit is the one
//! every desktop application has; the update trio is reached from the update
//! notification and from Settings → About.

gpui_kit::actions!(
    app,
    [
        /// Quit the application.
        Quit,
        /// Ask GitHub whether a newer release exists.
        CheckForUpdates,
        /// Download, verify and install the release the last check found.
        InstallUpdate,
        /// Start the installed release and quit this one.
        RestartToUpdate,
    ]
);
