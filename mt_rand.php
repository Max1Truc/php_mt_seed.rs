<?php
mt_srand(0);
echo mt_rand() . "\n";

echo "\n";

mt_srand(0);
echo mt_rand(0, 4294967295) / 2 . "\n";

echo "\n";

mt_srand(4242);
echo mt_rand(0, 4294967295) . "\n";
echo mt_rand(0, 4294967295) . "\n";
echo mt_rand(0, 4294967295) . "\n";

echo "\n";

mt_srand(424242);
echo mt_rand(1000, 10000) . "\n";
echo mt_rand(1000, 10000) . "\n";
echo mt_rand(1000, 10000) . "\n";