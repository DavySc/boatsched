import { Login } from "./Login";
import { SessionProvider } from "./session";
import { Shell } from "./Shell";
import { ToastProvider } from "./ui/Toast";

export function App() {
  return (
    <ToastProvider>
      <SessionProvider login={(onDone) => <Login onDone={onDone} />}>
        <Shell />
      </SessionProvider>
    </ToastProvider>
  );
}
