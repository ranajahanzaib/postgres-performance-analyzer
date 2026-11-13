DO $$
DECLARE current_status text;
BEGIN
  SELECT status INTO current_status FROM orders WHERE id = 1;
  IF current_status IS DISTINCT FROM 'complete' THEN
    RAISE EXCEPTION 'hot order status is %, expected complete', current_status;
  END IF;
  RAISE NOTICE 'hot order status is complete';
END $$;
