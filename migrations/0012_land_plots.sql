-- ROADMAP 34：城外產權地塊。只存「誰擁有哪塊」的稀疏表，未購地塊不在此表。
-- plot_id 對應 src/land_plot.rs 中 LAND_PLOTS 的 0..19；不用外鍵（靜態清單）。
CREATE TABLE IF NOT EXISTS land_plots (
    plot_id   INTEGER PRIMARY KEY,
    owner_id  UUID    NOT NULL
);
