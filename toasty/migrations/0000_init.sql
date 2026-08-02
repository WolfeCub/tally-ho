CREATE TABLE "receipts" (
    "id" BLOB NOT NULL,
    "purchased_on" TEXT NOT NULL,
    "merchant" TEXT NOT NULL,
    "subtotal" TEXT,
    "tax" TEXT,
    "total" TEXT,
    "currency" TEXT NOT NULL,
    "image_path" TEXT NOT NULL,
    "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'extracting', 'done', 'failed')),
    "extraction_error" TEXT,
    "model_used" TEXT,
    "raw_response" TEXT,
    "reviewed_at" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_receipts_by_purchased_on" ON "receipts" ("purchased_on");
-- #[toasty::breakpoint]
CREATE TABLE "line_items" (
    "id" BLOB NOT NULL,
    "receipt_id" BLOB NOT NULL,
    "description" TEXT NOT NULL,
    "quantity" TEXT,
    "unit_price" TEXT,
    "total" TEXT NOT NULL,
    "position" BIGINT NOT NULL,
    "edited" BOOLEAN NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_line_items_by_receipt_id" ON "line_items" ("receipt_id");
