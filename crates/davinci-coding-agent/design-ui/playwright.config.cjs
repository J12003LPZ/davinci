const {defineConfig}=require('@playwright/test');
module.exports=defineConfig({testDir:'./e2e',fullyParallel:false,workers:1,timeout:30000,
  use:{browserName:'chromium',headless:true,viewport:{width:1600,height:1000}},
  reporter:'list',outputDir:'test-results'});
