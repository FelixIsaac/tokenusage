import { motion } from "framer-motion";

const CARDS = [
  {
    title: "Daily card",
    desc: "Hourly distribution for the current day",
    cmd: "tu img day",
    img: "/assets/media/share-demo.png",
    alt: "Daily share card",
  },
  {
    title: "Weekly card",
    desc: "Day-by-day trend for the last 7 days",
    cmd: "tu img week",
    img: "/assets/media/share-week-demo.png",
    alt: "Weekly share card",
  },
];

export default function ShareCards() {
  return (
    <section id="share" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          Share
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(2rem,4vw,3.5rem)] leading-none">
          Cards people can actually post.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          tokenusage already generates dark social cards for daily and weekly usage. The website is the natural
          place to explain them with richer motion later, while the product keeps producing the images.
        </p>
      </motion.div>

      <div className="grid grid-cols-1 gap-5.5 md:grid-cols-2">
        {CARDS.map((card, i) => (
          <motion.article
            key={card.cmd}
            className="glass p-5.5 group"
            initial={{ opacity: 0, y: 28 }}
            whileInView={{ opacity: 1, y: 0 }}
            viewport={{ once: true, margin: "-5%" }}
            transition={{ duration: 0.5, delay: i * 0.1 }}
          >
            <div className="flex items-center justify-between gap-4.5 mb-3.5">
              <div>
                <h3 className="m-0 mb-1 font-[family-name:var(--font-display)] text-2xl">{card.title}</h3>
                <p className="m-0 text-text-dim text-[0.86rem]">{card.desc}</p>
              </div>
              <code className="shrink-0 px-2.5 py-2 rounded-[10px] bg-[rgba(8,17,30,0.82)] border border-[rgba(116,150,198,0.2)] text-cyan text-sm">
                {card.cmd}
              </code>
            </div>
            <img
              src={card.img}
              alt={card.alt}
              className="mt-4.5 w-full rounded-[18px] border border-[rgba(101,122,165,0.18)] transition-transform duration-300 group-hover:scale-[1.02]"
            />
          </motion.article>
        ))}
      </div>
    </section>
  );
}
