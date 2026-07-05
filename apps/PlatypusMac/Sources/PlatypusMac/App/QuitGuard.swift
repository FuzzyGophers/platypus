// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
import AppKit
import SwiftUI

/// Shared bridge between the editor's staged, unsaved changes and the app-level quit
/// handler, and the **owner of the red-✕ override**.
///
/// The editor's dirty state lives entirely in `CatalogView`'s private `@State`;
/// `CatalogView` writes `isDirty` / `summary` here every render (see its `body`) and
/// `AppDelegate.applicationShouldTerminate` reads them to warn. Nothing is written to
/// the card until **Save**, so quitting with staged changes silently loses them without
/// this guard.
///
/// `QuitGuard` is also the **target** of the window's red close button (see
/// `WindowAccessor`): the button's action is re-pointed to `requestQuit` so the ✕
/// becomes a *quit request* (routed through `applicationShouldTerminate`, which is
/// cancellable and doesn't tear the window down) instead of an un-cancellable SwiftUI
/// window close. It must be an `NSObject` to serve as a control target, and the singleton
/// is retained forever — which matters because `NSControl.target` is a *weak* reference.
final class QuitGuard: NSObject {
    static let shared = QuitGuard()
    private override init() {}

    /// True when there are staged changes not yet written to the card.
    var isDirty = false
    /// A short human description of the staged changes (e.g. "1 edited · 1 settings").
    var summary = ""

    /// Keep the guard current with the editor's staged state. Called from the view's
    /// `body` so it always reflects the latest render (deterministic — no dependence on
    /// `onChange` firing for a computed value). Writing to this plain class from `body`
    /// is safe: it isn't SwiftUI state, so it triggers no re-render.
    func update(dirty: Bool, summary: String) {
        isDirty = dirty
        self.summary = summary
    }

    /// The red ✕'s action is re-pointed here (see `WindowAccessor`), so clicking it
    /// *requests a quit* rather than closing the window. A quit request goes through
    /// `applicationShouldTerminate`, which can be cleanly cancelled — and cancelling it
    /// leaves the window (and its staged edits) exactly where they were, because nothing
    /// closed it. `⌘Q` / Quit menu / Dock-quit already take this same path.
    @objc func requestQuit() {
        NSApp.terminate(nil)
    }
}

/// App delegate: warn before losing staged edits on quit. Installed via
/// `@NSApplicationDelegateAdaptor`. Note SwiftUI wraps this in its own internal
/// `SwiftUI.AppDelegate` and puts *that* at `NSApp.delegate` (forwarding
/// `NSApplicationDelegate` methods here) — so never try to fetch this instance back off
/// `NSApp.delegate`; the red-✕ override targets `QuitGuard.shared` instead.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard QuitGuard.shared.isDirty else { return .terminateNow }
        // Bring the app to the front so the alert isn't hidden behind another app's
        // window (e.g. quitting from the Dock while a terminal is frontmost).
        NSApp.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "You have unsaved changes"
        let summary = QuitGuard.shared.summary
        alert.informativeText =
            (summary.isEmpty ? "Your staged changes " : "Your staged changes (\(summary)) ")
            + "haven't been written to the card. Quitting discards them — Cancel and Save "
            + "first, or discard and quit."
        alert.addButton(withTitle: "Cancel")  // default (Return / Esc)
        alert.addButton(withTitle: "Discard & Quit")
        return alert.runModal() == .alertSecondButtonReturn ? .terminateNow : .terminateCancel
    }
}

/// Re-points the window's red close button to `QuitGuard.shared.requestQuit`, turning the
/// ✕ from an un-cancellable SwiftUI window close into a cancellable quit request. The
/// window doesn't exist at make-time and `updateNSView` isn't guaranteed to fire once it
/// appears, so we poll until it's up. Drop into the view tree as
/// `.background(WindowAccessor())`.
struct WindowAccessor: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        retryRepoint(from: view, attemptsLeft: 300)  // ~30s — survives the SD-card Allow prompt
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        repointCloseButton(from: nsView)
    }

    private func retryRepoint(from view: NSView, attemptsLeft: Int) {
        guard attemptsLeft > 0 else { return }
        if view.window != nil {
            repointCloseButton(from: view)
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            retryRepoint(from: view, attemptsLeft: attemptsLeft - 1)
        }
    }

    private func repointCloseButton(from view: NSView) {
        guard let window = view.window,
            let closeButton = window.standardWindowButton(.closeButton),
            closeButton.action != #selector(QuitGuard.requestQuit)
        else { return }
        closeButton.target = QuitGuard.shared
        closeButton.action = #selector(QuitGuard.requestQuit)
    }
}
