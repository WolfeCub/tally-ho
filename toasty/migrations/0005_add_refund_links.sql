ALTER TABLE "charges" ADD COLUMN "refunds_charge_id" BLOB;
-- #[toasty::breakpoint]
CREATE INDEX "index_charges_by_refunds_charge_id" ON "charges" ("refunds_charge_id");
