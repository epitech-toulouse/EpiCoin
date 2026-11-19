-- Cohorts become a closed list (`models::COHORTS`). A value that names one of them,
-- however it was typed ("PGE 1", "MSc-2027", "Bachelor 2"), takes its code. Any other value
-- is kept as is: those students are listed under "sans cohorte" until an admin assigns one.
UPDATE users
SET promo = regexp_replace(regexp_replace(lower(promo), '[\s_-]', '', 'g'), '^bachelor', 'bach')
WHERE regexp_replace(regexp_replace(lower(promo), '[\s_-]', '', 'g'), '^bachelor', 'bach')
      IN ('msc2027', 'msc2028', 'msc2029', 'pge1', 'pge2', 'pge3', 'bach1', 'bach2', 'bach3');
