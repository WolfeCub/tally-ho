ALTER TABLE "line_items" ADD COLUMN "guessed_why" TEXT;
-- #[toasty::breakpoint]
DROP INDEX "index_receipts_by_purchased_on";
-- #[toasty::breakpoint]
PRAGMA foreign_keys = OFF;
-- #[toasty::breakpoint]
CREATE TABLE "_toasty_new_receipts" (
    "id" BLOB NOT NULL,
    "purchased_on" TEXT NOT NULL,
    "merchant" TEXT NOT NULL,
    "subtotal" TEXT,
    "tax" TEXT,
    "total" TEXT,
    "currency" TEXT NOT NULL,
    "image_path" TEXT NOT NULL,
    "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'extracting', 'assigning', 'done', 'failed')),
    "extraction_error" TEXT,
    "model_used" TEXT,
    "raw_response" TEXT,
    "reviewed_at" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
INSERT INTO "_toasty_new_receipts" ("id", "purchased_on", "merchant", "subtotal", "tax", "total", "currency", "image_path", "status", "extraction_error", "model_used", "raw_response", "reviewed_at", "created_at", "updated_at") SELECT "id", "purchased_on", "merchant", "subtotal", "tax", "total", "currency", "image_path", "status", "extraction_error", "model_used", "raw_response", "reviewed_at", "created_at", "updated_at" FROM "receipts";
-- #[toasty::breakpoint]
DROP TABLE "receipts";
-- #[toasty::breakpoint]
ALTER TABLE "_toasty_new_receipts" RENAME TO "receipts";
-- #[toasty::breakpoint]
PRAGMA foreign_keys = ON;
