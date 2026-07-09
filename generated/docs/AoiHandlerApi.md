# \AoiHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**aoi_list_handler**](AoiHandlerApi.md#aoi_list_handler) | **GET** /api/v1/aois | 
[**aoi_list_search_handler**](AoiHandlerApi.md#aoi_list_search_handler) | **GET** /api/v1/aois/search | 
[**create_aoi_handler**](AoiHandlerApi.md#create_aoi_handler) | **POST** /api/v1/aois | 
[**delete_aoi_handler**](AoiHandlerApi.md#delete_aoi_handler) | **DELETE** /api/v1/aois/{id} | 
[**edit_aoi_handler**](AoiHandlerApi.md#edit_aoi_handler) | **PATCH** /api/v1/aois/{id} | 
[**get_aoi_handler**](AoiHandlerApi.md#get_aoi_handler) | **GET** /api/v1/aois/{id} | 
[**get_aoi_pixel_ratio_handler**](AoiHandlerApi.md#get_aoi_pixel_ratio_handler) | **GET** /api/v1/aois/{id}/pixel-ratio | 



## aoi_list_handler

> models::PagedResponseAoiModelExtended aoi_list_handler(page, limit, order_by, desc, name, user_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | AoI name |  |
**user_id** | Option<**String**> | user ID |  |

### Return type

[**models::PagedResponseAoiModelExtended**](PagedResponse_AoiModelExtended.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## aoi_list_search_handler

> models::PagedResponseAoiModelExtended aoi_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | search by aois's name, mmpi_id or user_id |  |

### Return type

[**models::PagedResponseAoiModelExtended**](PagedResponse_AoiModelExtended.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## create_aoi_handler

> models::AoiModelExtended create_aoi_handler(create_aoi_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_aoi_schema** | [**CreateAoiSchema**](CreateAoiSchema.md) | Create Aoi | [required] |

### Return type

[**models::AoiModelExtended**](AoiModelExtended.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_aoi_handler

> delete_aoi_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Aoi id for deleting aoi | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## edit_aoi_handler

> models::AoiModelExtended edit_aoi_handler(id, update_aoi_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Aoi id for patching aoi | [required] |
**update_aoi_schema** | [**UpdateAoiSchema**](UpdateAoiSchema.md) | Update Aoi | [required] |

### Return type

[**models::AoiModelExtended**](AoiModelExtended.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_aoi_handler

> models::AoiModelExtended get_aoi_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Aoi id for fetching aoi | [required] |

### Return type

[**models::AoiModelExtended**](AoiModelExtended.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_aoi_pixel_ratio_handler

> models::TargetBaselineResponseModel get_aoi_pixel_ratio_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Aoi id | [required] |

### Return type

[**models::TargetBaselineResponseModel**](TargetBaselineResponseModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

