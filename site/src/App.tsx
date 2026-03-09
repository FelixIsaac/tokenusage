import Header from "./components/Header";
import Hero from "./components/Hero";
import MetricsStrip from "./components/MetricsStrip";
import Tour from "./components/Tour";
import Install from "./components/Install";
import Docs from "./components/Docs";
import Footer from "./components/Footer";

export default function App() {
  return (
    <div className="relative min-h-screen overflow-clip">
      {/* Background layers */}
      <div className="fixed inset-0 bg-grid pointer-events-none" aria-hidden="true" />
      <div
        className="fixed top-20 -left-32 w-[34rem] h-[34rem] rounded-full opacity-30 blur-3xl pointer-events-none"
        style={{ background: "radial-gradient(circle, rgba(93,231,226,0.36), transparent 72%)" }}
        aria-hidden="true"
      />
      <div
        className="fixed top-80 -right-32 w-[34rem] h-[34rem] rounded-full opacity-25 blur-3xl pointer-events-none"
        style={{ background: "radial-gradient(circle, rgba(130,245,180,0.22), transparent 68%)" }}
        aria-hidden="true"
      />

      <Header />

      <main>
        <Hero />
        <MetricsStrip />
        <Tour />
        <Install />
        <Docs />
        <Footer />
      </main>
    </div>
  );
}
