import SwiftUI

@main struct JianxinApp: App {
    @StateObject private var store = MailStore()
    var body: some Scene {
        WindowGroup { RootView().environmentObject(store).tint(MailTheme.primary).preferredColorScheme(.light).task{await store.bootstrap()} }
    }
}
